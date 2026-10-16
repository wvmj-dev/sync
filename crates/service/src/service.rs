use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Instant;

use syncly_core::CoreError;
use syncly_core::config::{AppConfig, Settings};
use syncly_core::domain::{
    Account, AccountKind, Delivery, Destination, GuardLimits, Job, Plan, RelPath, RunKind,
    RunStatus, RunTrigger, Schedule, SkipReason, SyncSettings, ValidationContext,
};
use syncly_core::filters::Filter;
use syncly_core::lock::JobLock;
use syncly_core::paths::AppPaths;
use syncly_core::provider::{ProviderError, StorageProvider};
use syncly_core::providers::LocalProvider;
use syncly_core::state::StateDb;
use syncly_core::sync::executor::{self, Execution, Progress};
use syncly_core::sync::guard::{self, GuardInput, GuardVerdict};
use syncly_core::sync::planner::{self, PlanInput};
use syncly_core::sync::remote_tree::{RemoteTree, load_remote_tree};
use syncly_core::sync::retry::RetryPolicy;
use syncly_core::sync::scanner::{self, ScanRequest, ScanResult};
use syncly_gdrive::HttpClient;
use syncly_gdrive::auth::{ClientConfigError, load_client};
use uuid::Uuid;

use crate::delivery::ArchiveExec;
use crate::dto::{
    AccountSummary, ActionView, AppInfo, JobDraft, JobSummary, MAX_PREVIEW_ACTIONS,
    MAX_PREVIEW_WARNINGS, PlanPreview, Preferences, RunReport, SyncSummary, human_bytes,
};
use crate::error::{ServiceError, ServiceResult};
use crate::google::GoogleBackend;
use crate::os::{LinuxDesktop, OsIntegration};
use crate::receive::ReceiveExec;
use crate::schedule;
use crate::secrets::{KeyringStore, SecretStore};

/// Ponto de entrada dos casos de uso. A configuração é relida do disco a cada
/// chamada (arquivo pequeno), então GUI e CLI sempre enxergam a mesma versão.
pub struct Service {
    pub(crate) paths: AppPaths,
    write_lock: Mutex<()>,
    pub(crate) secrets: Arc<dyn SecretStore>,
    pub(crate) google: GoogleBackend,
    pub(crate) http: OnceLock<HttpClient>,
    /// Execuções em andamento neste processo → sinal de cancelamento.
    pub(crate) running: Mutex<HashMap<Uuid, Arc<AtomicBool>>>,
    /// Compactação/descompactação em andamento (uma por vez).
    pub(crate) archive_op: Mutex<Option<Arc<AtomicBool>>>,
    /// systemd e notificações (RN-80 a RN-83, RN-94).
    pub(crate) os: Arc<dyn OsIntegration>,
}

/// Tarefa lida do disco e validada, com a conta de destino.
pub(crate) struct Loaded {
    pub job: Job,
    pub sync: SyncSettings,
    pub settings: Settings,
    pub account: Account,
}

/// Resultado da preparação de uma execução arquivo por arquivo.
struct Prepared {
    job: Job,
    sync: SyncSettings,
    settings: Settings,
    provider: Box<dyn StorageProvider>,
    scan: ScanResult,
    remote: RemoteTree,
    plan: Plan,
    verdict: GuardVerdict,
    first_run: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct RunOptions {
    /// RN-36, RN-42: o usuário revisou e confirmou o plano.
    pub confirm: bool,
    pub trigger: RunTrigger,
}

/// Remove a execução do mapa ao terminar, mesmo com erro.
pub(crate) struct RunningGuard<'a> {
    map: &'a Mutex<HashMap<Uuid, Arc<AtomicBool>>>,
    id: Uuid,
}

impl<'a> RunningGuard<'a> {
    pub(crate) fn register(
        map: &'a Mutex<HashMap<Uuid, Arc<AtomicBool>>>,
        id: Uuid,
        flag: Arc<AtomicBool>,
    ) -> Self {
        map.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, flag);
        Self { map, id }
    }
}

impl Drop for RunningGuard<'_> {
    fn drop(&mut self) {
        self.map
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
    }
}

pub(crate) fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

impl Service {
    /// Produção: keyring do sistema, endpoints reais do Google e systemd do usuário.
    pub fn open(paths: AppPaths) -> ServiceResult<Self> {
        Self::with_backends(
            paths,
            Arc::new(KeyringStore),
            GoogleBackend::default(),
            Arc::new(LinuxDesktop),
        )
    }

    /// Permite trocar o cofre, o Google e o sistema — os testes usam memória, um servidor
    /// simulado e um systemd de mentira.
    pub fn with_backends(
        paths: AppPaths,
        secrets: Arc<dyn SecretStore>,
        google: GoogleBackend,
        os: Arc<dyn OsIntegration>,
    ) -> ServiceResult<Self> {
        paths.ensure()?;
        Ok(Self {
            paths,
            write_lock: Mutex::new(()),
            secrets,
            google,
            http: OnceLock::new(),
            running: Mutex::new(HashMap::new()),
            archive_op: Mutex::new(None),
            os,
        })
    }

    pub fn open_default() -> ServiceResult<Self> {
        Self::open(AppPaths::discover()?)
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    pub(crate) fn load(&self) -> ServiceResult<AppConfig> {
        Ok(AppConfig::load(&self.paths.config_file())?)
    }

    /// Lê → altera → valida → grava atomicamente. Uma alteração inválida nunca chega ao disco.
    pub(crate) fn mutate<T>(
        &self,
        f: impl FnOnce(&mut AppConfig) -> ServiceResult<T>,
    ) -> ServiceResult<T> {
        let _guard = self
            .write_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut config = self.load()?;
        let out = f(&mut config)?;
        config.save(&self.paths.config_file(), &self.paths.app_dirs())?;
        tracing::info!("configuração salva");
        Ok(out)
    }

    pub(crate) fn state(&self) -> ServiceResult<StateDb> {
        Ok(StateDb::open(&self.paths.state_db())?)
    }

    // ---- Informações ----

    pub fn app_info(&self) -> AppInfo {
        let (google_client_configured, google_client_problem) =
            match load_client(&self.paths.google_client_file()) {
                Ok(_) => (true, None),
                Err(ClientConfigError::Missing(_)) => (false, None),
                Err(e) => (false, Some(e.to_string())),
            };
        AppInfo {
            version: env!("CARGO_PKG_VERSION"),
            config_file: self.paths.config_file().display().to_string(),
            data_dir: self.paths.data_dir.display().to_string(),
            log_dir: self.paths.log_dir.display().to_string(),
            google_client_configured,
            google_client_problem,
            config_problem: self.load().err().map(|e| e.to_string()),
        }
    }

    // ---- Preferências ----

    pub fn preferences(&self) -> ServiceResult<Preferences> {
        let s = self.load()?.settings;
        Ok(Preferences {
            notify: s.notify,
            min_free_space_mib: s.min_free_space_mib,
            max_parallel_transfers: s.max_parallel_transfers,
        })
    }

    /// Grava as preferências; valores fora dos limites são recusados sem tocar no arquivo.
    pub fn save_preferences(&self, p: Preferences) -> ServiceResult<Preferences> {
        self.mutate(|config| {
            config.settings.notify = p.notify;
            config.settings.min_free_space_mib = p.min_free_space_mib;
            config.settings.max_parallel_transfers = p.max_parallel_transfers;
            Ok(())
        })?;
        self.preferences()
    }

    // ---- Contas ----

    pub fn list_accounts(&self) -> ServiceResult<Vec<AccountSummary>> {
        let config = self.load()?;
        let reconnect = self.state()?.accounts_needing_reconnect()?;
        Ok(config
            .accounts
            .iter()
            .map(|a| account_summary(&config, a, reconnect.get(&a.id).copied()))
            .collect())
    }

    /// RN-07: pasta local existente, marcada como raiz do Syncly.
    pub fn add_local_account(&self, label: String, root: PathBuf) -> ServiceResult<AccountSummary> {
        if !root.is_absolute() || !root.is_dir() {
            return Err(ServiceError::Invalid(format!(
                "'{}' não é uma pasta existente",
                root.display()
            )));
        }
        if self.paths.app_dirs().iter().any(|d| root.starts_with(d)) {
            return Err(ServiceError::Invalid(
                "a pasta não pode ficar dentro das pastas do Syncly".into(),
            ));
        }
        // Cria o marcador primeiro: também confirma que a pasta é gravável.
        LocalProvider::initialize(&root).map_err(CoreError::from)?;
        let account = Account {
            id: Uuid::new_v4(),
            label: label.trim().to_string(),
            kind: AccountKind::Local { root: root.clone() },
        };
        let saved = account.clone();
        self.mutate(move |config| {
            if config
                .accounts
                .iter()
                .any(|a| matches!(&a.kind, AccountKind::Local { root: r } if *r == root))
            {
                return Err(ServiceError::Invalid("essa pasta já é uma conta".into()));
            }
            config.accounts.push(account);
            Ok(())
        })?;
        Ok(account_summary(&self.load()?, &saved, None))
    }

    /// RN-05: as tarefas que usavam a conta são desativadas, não apagadas.
    /// Contas Google têm o acesso revogado e a credencial apagada do cofre (SEG-06).
    pub async fn remove_account(&self, id: Uuid) -> ServiceResult<()> {
        let account = self
            .load()?
            .account(id)
            .cloned()
            .ok_or(ServiceError::NotFound("conta"))?;
        if matches!(account.kind, AccountKind::GoogleDrive { .. }) {
            self.forget_google_credentials(id).await?;
        }
        self.mutate(|config| {
            let before = config.accounts.len();
            config.accounts.retain(|a| a.id != id);
            if config.accounts.len() == before {
                return Err(ServiceError::NotFound("conta"));
            }
            for job in &mut config.jobs {
                if job
                    .sync
                    .as_ref()
                    .is_some_and(|s| s.destination.account_id == id)
                {
                    job.enabled = false;
                }
            }
            Ok(())
        })?;
        self.state()?.clear_needs_reconnect(id)?;
        // RN-05, RN-18: as tarefas desativadas perdem o timer.
        self.refresh_schedules();
        Ok(())
    }

    // ---- Tarefas ----

    pub fn list_jobs(&self) -> ServiceResult<Vec<JobSummary>> {
        let config = self.load()?;
        let state = self.state()?;
        let unit_dir = self.paths.systemd_unit_dir();
        let mut jobs = config
            .jobs
            .iter()
            .map(|j| job_summary(&config, &state, &unit_dir, j))
            .collect::<ServiceResult<Vec<_>>>()?;
        self.fill_next_runs(&mut jobs);
        Ok(jobs)
    }

    /// RN-100: próximo disparo dos timers ativos, numa única consulta ao systemd. Sem
    /// systemd, a informação só fica de fora.
    fn fill_next_runs(&self, jobs: &mut [JobSummary]) {
        let timer = |id: Uuid| format!("{}.timer", syncly_core::timers::unit_base(id));
        let active: Vec<String> = jobs
            .iter()
            .filter(|j| {
                j.sync
                    .as_ref()
                    .is_some_and(|s| s.schedule_state == "active")
            })
            .map(|j| timer(j.id))
            .collect();
        match self.os.next_runs(&active) {
            Ok(next) => {
                for job in jobs.iter_mut() {
                    if let Some(sync) = job.sync.as_mut() {
                        sync.next_run = next.get(&timer(job.id)).copied();
                    }
                }
            }
            Err(e) => tracing::debug!("próximas execuções indisponíveis: {e}"),
        }
    }

    pub fn find_job_id(&self, key: &str) -> ServiceResult<Uuid> {
        self.load()?
            .find_job(key)
            .map(|j| j.id)
            .ok_or(ServiceError::NotFound("tarefa"))
    }

    pub fn save_job(&self, draft: JobDraft) -> ServiceResult<JobSummary> {
        let sync = match &draft.sync {
            Some(s) => Some((
                s.account_id,
                RelPath::parse(&s.path).map_err(ServiceError::Invalid)?,
                s.delivery.clone(),
                s.mode,
                s.schedule.clone(),
            )),
            None => None,
        };
        // RN-82: a expressão avançada também passa pelo próprio systemd antes de salvar.
        if let Some((.., Some(schedule @ Schedule::Calendar { expr }))) = &sync {
            schedule.validate().map_err(ServiceError::Invalid)?;
            self.os.verify_calendar(expr.trim())?;
        }
        let id = self.mutate(|config| {
            let id = draft.id.unwrap_or_else(Uuid::new_v4);
            let existing = config.jobs.iter().position(|j| j.id == id);
            if draft.id.is_some() && existing.is_none() {
                return Err(ServiceError::NotFound("tarefa"));
            }
            let previous_sync = existing.and_then(|i| config.jobs[i].sync.clone());
            let job = Job {
                id,
                name: draft.name.trim().to_string(),
                enabled: draft.enabled,
                sources: draft.sources.clone(),
                exclude: clean_patterns(&draft.exclude),
                include_sensitive: draft.include_sensitive,
                sync: sync.map(
                    |(account_id, path, delivery, mode, schedule)| SyncSettings {
                        destination: Destination { account_id, path },
                        delivery,
                        mode,
                        schedule: schedule.unwrap_or_else(|| {
                            previous_sync
                                .as_ref()
                                .map_or(Schedule::Manual, |s| s.schedule.clone())
                        }),
                        guard: previous_sync
                            .as_ref()
                            .map_or(GuardLimits::default(), |s| s.guard),
                    },
                ),
            };
            match existing {
                Some(i) => config.jobs[i] = job,
                None => config.jobs.push(job),
            }
            Ok(id)
        })?;
        self.refresh_schedules();
        let config = self.load()?;
        let job = config.job(id).ok_or(ServiceError::NotFound("tarefa"))?;
        let mut summary = [job_summary(
            &config,
            &self.state()?,
            &self.paths.systemd_unit_dir(),
            job,
        )?];
        self.fill_next_runs(&mut summary);
        let [summary] = summary;
        Ok(summary)
    }

    pub fn delete_job(&self, id: Uuid) -> ServiceResult<()> {
        self.mutate(|config| {
            let before = config.jobs.len();
            config.jobs.retain(|j| j.id != id);
            if config.jobs.len() == before {
                return Err(ServiceError::NotFound("tarefa"));
            }
            Ok(())
        })?;
        // O histórico de execuções fica; o índice não serve para mais nada.
        self.state()?.clear_job(id)?;
        // RN-18: sem tarefa, sem timer.
        self.refresh_schedules();
        Ok(())
    }

    // ---- Sincronização ----

    /// Lê e valida a tarefa. O config.toml pode ter sido editado à mão: valida antes de
    /// tocar em qualquer arquivo.
    pub(crate) fn load_job(&self, id: Uuid) -> ServiceResult<Loaded> {
        let config = self.load()?;
        let job = config
            .job(id)
            .cloned()
            .ok_or(ServiceError::NotFound("tarefa"))?;
        let app_dirs = self.paths.app_dirs();
        let mut issues = job.validate(&ValidationContext {
            app_dirs: &app_dirs,
        });
        issues.extend(config.validate_local_destination(&job));
        if !issues.is_empty() {
            return Err(CoreError::Invalid(issues).into());
        }
        let sync = job.sync.clone().ok_or_else(|| {
            ServiceError::Invalid("esta tarefa não tem destino configurado".into())
        })?;
        let account = config
            .account(sync.destination.account_id)
            .cloned()
            .ok_or(ServiceError::NotFound("conta de destino"))?;
        Ok(Loaded {
            job,
            sync,
            settings: config.settings,
            account,
        })
    }

    /// Varre, lista o destino, planeja e avalia as guardas — sem alterar nada.
    /// Compartilhado pela prévia e pela execução, para as duas nunca divergirem.
    async fn prepare(&self, loaded: Loaded) -> ServiceResult<Prepared> {
        let Loaded {
            job,
            sync,
            settings,
            account,
        } = loaded;
        let provider = self.provider_for(&account).await?;

        let state = self.state()?;
        let index = state.load_index(job.id)?;
        let first_run = !state.has_successful_run(job.id, RunKind::Sync)?;
        drop(state);

        let request = ScanRequest {
            sources: job.sources.clone(),
            filter: Filter::new(&job.exclude, job.include_sensitive)
                .map_err(ServiceError::Invalid)?,
            skip_dirs: self.paths.app_dirs(),
            hash_kind: Some(provider.hash_kind()),
        };
        let scan_index = index.clone();
        let scan = tokio::task::spawn_blocking(move || scanner::scan(&request, &scan_index))
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        let remote = load_remote_tree(provider.as_ref(), &sync.destination.path).await?;

        let mut blocked: BTreeMap<String, SkipReason> = scan
            .blocked
            .iter()
            .map(|p| (p.clone(), SkipReason::LocalUnreadable))
            .collect();
        blocked.extend(remote.blocked.clone());

        let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M").to_string();
        let plan = planner::plan(&PlanInput {
            mode: sync.mode,
            local: &scan.entries,
            index: &index,
            remote: &remote.entries,
            blocked: &blocked,
            excluded: &scan.excluded,
            conflict_stamp: &stamp,
        });
        let verdict = guard::evaluate(&GuardInput {
            sources: &scan.sources,
            index: &index,
            plan: &plan,
            limits: sync.guard,
            first_run,
        });
        Ok(Prepared {
            job,
            sync,
            settings,
            provider,
            scan,
            remote,
            plan,
            verdict,
            first_run,
        })
    }

    /// Prévia: o que uma execução faria agora.
    pub async fn preview_job(&self, id: Uuid) -> ServiceResult<PlanPreview> {
        let started = Instant::now();
        let loaded = self.load_job(id)?;
        match loaded.sync.delivery.clone() {
            Delivery::Archive {
                format,
                retention,
                encrypt,
            } => {
                return self
                    .preview_archive(loaded, format, retention, encrypt, started)
                    .await;
            }
            Delivery::Receive { target } => {
                return self.preview_receive(loaded, target, started).await;
            }
            Delivery::Files => {}
        }
        let p = self.prepare(loaded).await?;
        let warnings = p
            .scan
            .warnings
            .iter()
            .map(|w| w.describe())
            .chain(
                p.remote
                    .invalid_names
                    .iter()
                    .map(|n| format!("nome remoto ignorado: {n}")),
            )
            .take(MAX_PREVIEW_WARNINGS)
            .collect();
        tracing::info!(
            job = %p.job.id,
            actions = p.plan.actions.len(),
            hashed = p.scan.hashed_files,
            "prévia calculada"
        );
        Ok(PlanPreview {
            job_id: p.job.id,
            mode: p.sync.mode,
            total_actions: p.plan.actions.len(),
            actions: p
                .plan
                .actions
                .iter()
                .take(MAX_PREVIEW_ACTIONS)
                .map(ActionView::from)
                .collect(),
            stats: p.plan.stats.clone(),
            verdict: p.verdict,
            warnings,
            hashed_files: p.scan.hashed_files,
            duration_ms: elapsed_ms(started),
            first_run: p.first_run,
            archive: None,
            receive: None,
        })
    }

    /// Executa a sincronização de uma tarefa (RN-36 a RN-46, RN-90 a RN-95).
    /// `progress` é chamado a cada item; a GUI repassa como evento, a CLI imprime.
    pub async fn run_job(
        &self,
        id: Uuid,
        options: RunOptions,
        progress: impl Fn(&Progress) + Send + Sync,
    ) -> ServiceResult<RunReport> {
        let result = self.execute_job(id, options, progress).await;
        let account = self.load().ok().and_then(|c| {
            let id = c.job(id)?.sync.as_ref()?.destination.account_id;
            c.account(id).cloned()
        });
        // RN-06: login recusado no meio do caminho vira o aviso de reconectar.
        let result = match (result, &account) {
            (Err(ServiceError::Core(CoreError::Provider(ProviderError::AuthExpired))), Some(a)) => {
                Err(ServiceError::NeedsReconnect(a.label.clone()))
            }
            (other, _) => other,
        };
        // RN-85: conta a reconectar pausa os agendamentos dela.
        if let Some(a) = &account
            && self
                .state()
                .and_then(|s| Ok(s.accounts_needing_reconnect()?))
                .is_ok_and(|m| m.contains_key(&a.id))
        {
            self.refresh_schedules();
        }
        if matches!(result, Err(ServiceError::Core(CoreError::JobLocked))) {
            // RN-90: disparo agendado com a tarefa já rodando — descartado e registrado.
            if options.trigger == RunTrigger::Scheduled {
                self.record_unstarted(id, RunStatus::Discarded, "a tarefa já estava em execução");
            }
            return result;
        }
        // RN-103: a execução mudou o destino — a foto antiga deixa de valer.
        // RN-93: e o histórico não cresce para sempre.
        let cleanup = self.state().and_then(|s| {
            s.clear_snapshot(id)?;
            s.prune_history(chrono::Utc::now())?;
            Ok(())
        });
        if let Err(e) = cleanup {
            tracing::warn!("limpeza depois da execução falhou: {e}");
        }
        // RN-36: a primeira execução manual bem-sucedida ativa o agendamento.
        if options.trigger == RunTrigger::Manual && result.is_ok() {
            self.refresh_schedules();
        }
        result
    }

    /// Registra no histórico um disparo agendado que não chegou a rodar: descartado porque a
    /// tarefa já estava em execução (RN-90) ou adiado (RN-84). Falhar aqui só gera um aviso
    /// no log.
    pub(crate) fn record_unstarted(&self, id: Uuid, status: RunStatus, message: &str) {
        let record = || -> ServiceResult<()> {
            let kind = self
                .load()?
                .job(id)
                .and_then(|j| j.sync.as_ref())
                .map_or(RunKind::Sync, |s| s.delivery.run_kind());
            let state = self.state()?;
            let run = state.start_run(id, kind, RunTrigger::Scheduled)?;
            state.finish_run(run, status, None, Some(message))?;
            Ok(())
        };
        if let Err(e) = record() {
            tracing::warn!("disparo agendado ({}) não registrado: {e}", status.as_str());
        }
    }

    async fn execute_job(
        &self,
        id: Uuid,
        options: RunOptions,
        progress: impl Fn(&Progress) + Send + Sync,
    ) -> ServiceResult<RunReport> {
        let started = Instant::now();
        // RN-90: uma execução por tarefa (vale entre GUI e CLI).
        let _lock = JobLock::try_acquire(&self.paths.lock_dir, id)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let _running = RunningGuard::register(&self.running, id, cancel.clone());

        let loaded = self.load_job(id)?;
        // RN-18
        if !loaded.job.enabled {
            return Err(ServiceError::Invalid("a tarefa está desativada".into()));
        }
        match loaded.sync.delivery.clone() {
            Delivery::Archive {
                format,
                retention,
                encrypt,
            } => {
                // RN-73: só a chave pública — o keyring nem é aberto.
                let encrypt = match encrypt {
                    true => Some(self.encryption_public_key()?),
                    false => None,
                };
                let exec = ArchiveExec {
                    format,
                    retention,
                    encrypt,
                    options,
                    cancel: &cancel,
                    progress: &progress,
                    started,
                };
                return self.run_archive(loaded, exec).await;
            }
            Delivery::Receive { target } => {
                let exec = ReceiveExec {
                    options,
                    cancel: &cancel,
                    progress: &progress,
                    started,
                };
                return self.run_receive(loaded, target, exec).await;
            }
            Delivery::Files => {}
        }
        let p = self.prepare(loaded).await?;
        let state = self.state()?;
        check_verdict(&state, id, RunKind::Sync, &p.verdict, options)?;
        if let Some(msg) =
            quota_shortfall(p.provider.as_ref(), &p.settings, p.plan.stats.bytes_to_send).await?
        {
            let run = state.start_run(id, RunKind::Sync, options.trigger)?;
            state.finish_run(run, RunStatus::Aborted, None, Some(&msg))?;
            return Err(ServiceError::Invalid(msg));
        }

        let run_id = state.start_run(id, RunKind::Sync, options.trigger)?;
        let state = Mutex::new(state);
        tracing::info!(job = %id, actions = p.plan.actions.len(), "execução iniciada");
        let result = executor::execute(
            p.provider.as_ref(),
            &state,
            &Execution {
                job_id: id,
                dest: &p.sync.destination.path,
                plan: &p.plan,
                scan: &p.scan,
                remote: &p.remote.entries,
                parallelism: usize::from(p.settings.max_parallel_transfers),
                retry: RetryPolicy {
                    max_attempts: 2,
                    base_delay: std::time::Duration::from_secs(1),
                    max_delay: std::time::Duration::from_secs(4),
                },
                cancel: &cancel,
                progress: &progress,
            },
        )
        .await;
        let state = state.into_inner().unwrap_or_else(PoisonError::into_inner);

        let report = match result {
            Ok(report) => report,
            Err(e) => {
                state.finish_run(run_id, RunStatus::Failed, None, Some(&e.to_string()))?;
                return Err(e.into());
            }
        };
        // RN-91
        let status = if report.aborted.is_some() {
            RunStatus::Aborted
        } else if report.cancelled {
            RunStatus::Cancelled
        } else if report.failures.is_empty() {
            RunStatus::Success
        } else if report.succeeded_items() > 0 {
            RunStatus::PartialSuccess
        } else {
            RunStatus::Failed
        };
        let stats = serde_json::to_value(&report).ok();
        state.finish_run(run_id, status, stats.as_ref(), report.aborted.as_deref())?;
        tracing::info!(job = %id, status = status.as_str(), "execução concluída");
        Ok(RunReport::new(status, report, elapsed_ms(started)))
    }

    /// RN-95: pede para parar; o que já foi confirmado fica no índice.
    pub fn cancel_run(&self, id: Uuid) -> bool {
        let running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        match running.get(&id) {
            Some(flag) => {
                flag.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }
}

/// Aplica o veredito das guardas antes de qualquer alteração. Bloqueios e execuções
/// agendadas que precisariam de confirmação ficam registrados no histórico.
pub(crate) fn check_verdict(
    state: &StateDb,
    id: Uuid,
    kind: RunKind,
    verdict: &GuardVerdict,
    options: RunOptions,
) -> ServiceResult<()> {
    match verdict {
        GuardVerdict::Abort(reasons) => {
            let run = state.start_run(id, kind, options.trigger)?;
            state.finish_run(run, RunStatus::Aborted, None, Some(&reasons.join("; ")))?;
            Err(ServiceError::Invalid(format!(
                "execução bloqueada: {}",
                reasons.join("; ")
            )))
        }
        // RN-43: execução agendada nunca confirma sozinha.
        GuardVerdict::NeedsConfirmation(reasons) if options.trigger == RunTrigger::Scheduled => {
            let run = state.start_run(id, kind, options.trigger)?;
            state.finish_run(
                run,
                RunStatus::AwaitingConfirmation,
                None,
                Some(&reasons.join("; ")),
            )?;
            Err(ServiceError::NeedsConfirmation(reasons.clone()))
        }
        // RN-36, RN-42
        GuardVerdict::NeedsConfirmation(reasons) if !options.confirm => {
            Err(ServiceError::NeedsConfirmation(reasons.clone()))
        }
        _ => Ok(()),
    }
}

/// RN-46: reserva de espaço na conta (o Gmail para de receber e-mails se encher).
/// `Some(motivo)` se `needed` bytes não cabem mantendo a reserva.
pub(crate) async fn quota_shortfall(
    provider: &dyn StorageProvider,
    settings: &Settings,
    needed: u64,
) -> ServiceResult<Option<String>> {
    if needed == 0 {
        return Ok(None);
    }
    let Some(free) = provider
        .quota()
        .await
        .map_err(CoreError::from)?
        .and_then(|q| q.free())
    else {
        return Ok(None);
    };
    let reserve = settings.min_free_space_mib.saturating_mul(1024 * 1024);
    if free >= needed.saturating_add(reserve) {
        return Ok(None);
    }
    Ok(Some(format!(
        "espaço insuficiente: a execução precisa de {} e a conta tem {} livres \
         (reserva mínima de {})",
        human_bytes(needed),
        human_bytes(free),
        human_bytes(reserve)
    )))
}

impl Service {
    /// Fábrica de provedores: o único lugar que conhece todos eles (ARQUITETURA.md §2).
    /// RN-06: conta marcada como "requer reconexão" não tenta a rede.
    pub(crate) async fn provider_for(
        &self,
        account: &Account,
    ) -> ServiceResult<Box<dyn StorageProvider>> {
        if self
            .state()?
            .accounts_needing_reconnect()?
            .contains_key(&account.id)
        {
            return Err(ServiceError::NeedsReconnect(account.label.clone()));
        }
        match &account.kind {
            AccountKind::Local { root } => Ok(Box::new(LocalProvider::new(root.clone()))),
            AccountKind::GoogleDrive { root_folder_id, .. } => {
                self.google_provider(account, root_folder_id.clone()).await
            }
        }
    }
}

fn clean_patterns(patterns: &[String]) -> Vec<String> {
    patterns
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty() && !p.starts_with('#'))
        .map(str::to_string)
        .collect()
}

pub(crate) fn account_summary(
    config: &AppConfig,
    account: &Account,
    needs_reconnect_since: Option<chrono::DateTime<chrono::Utc>>,
) -> AccountSummary {
    let detail = match &account.kind {
        AccountKind::GoogleDrive { email, .. } => email.clone().unwrap_or_default(),
        AccountKind::Local { root } => root.display().to_string(),
    };
    let job_count = config
        .jobs
        .iter()
        .filter(|j| {
            j.sync
                .as_ref()
                .is_some_and(|s| s.destination.account_id == account.id)
        })
        .count();
    AccountSummary {
        id: account.id,
        label: account.label.clone(),
        provider: account.kind.provider_id(),
        detail,
        job_count,
        needs_reconnect_since,
    }
}

fn job_summary(
    config: &AppConfig,
    state: &StateDb,
    unit_dir: &Path,
    job: &Job,
) -> ServiceResult<JobSummary> {
    let schedule_state = schedule::timer_state(state, unit_dir, job)?;
    Ok(JobSummary {
        id: job.id,
        name: job.name.clone(),
        enabled: job.enabled,
        sources: job
            .sources
            .iter()
            .map(|s| s.display().to_string())
            .collect(),
        exclude: job.exclude.clone(),
        include_sensitive: job.include_sensitive,
        sync: job.sync.as_ref().map(|s| SyncSummary {
            account_id: s.destination.account_id,
            account_label: config
                .account(s.destination.account_id)
                .map(|a| a.label.clone()),
            path: s.destination.path.to_string(),
            delivery: s.delivery.clone(),
            mode: s.mode,
            schedule: s.schedule.label(),
            schedule_spec: s.schedule.clone(),
            schedule_state: schedule_state.as_str(),
            next_run: None,
        }),
        last_run: state.last_run(job.id)?.map(Into::into),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::SyncDraft;
    use syncly_core::domain::SyncMode;
    use syncly_core::sync::guard::GuardVerdict;

    fn setup() -> (tempfile::TempDir, Service, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let service = crate::test_support::service(&tmp.path().join("app"), "http://127.0.0.1:9");
        let source = tmp.path().join("Docs");
        std::fs::create_dir_all(source.join("sub")).unwrap();
        std::fs::write(source.join("a.txt"), "a").unwrap();
        std::fs::write(source.join("sub/b.txt"), "b").unwrap();
        let dest = tmp.path().join("hd");
        std::fs::create_dir_all(&dest).unwrap();
        (tmp, service, source, dest)
    }

    #[tokio::test]
    async fn fluxo_conta_local_tarefa_e_previa() {
        let (_tmp, service, source, dest) = setup();
        let account = service
            .add_local_account("HD externo".into(), dest.clone())
            .unwrap();
        let job = service
            .save_job(JobDraft {
                id: None,
                name: "Documentos".into(),
                enabled: true,
                sources: vec![source],
                exclude: vec!["  ".into(), "*.log".into()],
                include_sensitive: false,
                sync: Some(SyncDraft {
                    account_id: account.id,
                    path: "/Backup/Docs/".into(),
                    delivery: Delivery::Files,
                    mode: SyncMode::Mirror,
                    schedule: None,
                }),
            })
            .unwrap();
        assert_eq!(job.exclude, ["*.log"]);
        assert_eq!(job.sync.as_ref().unwrap().path, "Backup/Docs");

        let preview = service.preview_job(job.id).await.unwrap();
        assert_eq!(preview.stats.uploads, 2);
        assert_eq!(preview.stats.folders_to_create, 2);
        assert!(preview.first_run);
        assert!(matches!(
            preview.verdict,
            GuardVerdict::NeedsConfirmation(_)
        ));
        // A prévia não alterou o destino.
        assert!(!dest.join("Backup").exists());

        assert_eq!(service.list_jobs().unwrap().len(), 1);
        assert_eq!(service.list_accounts().unwrap()[0].job_count, 1);
    }

    fn job_to_local(service: &Service, source: PathBuf, dest: PathBuf) -> Uuid {
        let account = service.add_local_account("HD".into(), dest).unwrap();
        service
            .save_job(JobDraft {
                id: None,
                name: "Docs".into(),
                enabled: true,
                sources: vec![source],
                exclude: vec![],
                include_sensitive: false,
                sync: Some(SyncDraft {
                    account_id: account.id,
                    path: "Backup".into(),
                    delivery: Delivery::Files,
                    mode: SyncMode::Upload,
                    schedule: None,
                }),
            })
            .unwrap()
            .id
    }

    fn manual(confirm: bool) -> RunOptions {
        RunOptions {
            confirm,
            trigger: RunTrigger::Manual,
        }
    }

    #[tokio::test]
    async fn rn36_execucao_exige_confirmacao_na_primeira_vez() {
        let (_tmp, service, source, dest) = setup();
        let id = job_to_local(&service, source, dest.clone());

        let err = service
            .run_job(id, manual(false), |_| {})
            .await
            .unwrap_err();
        assert_eq!(err.code(), "needs_confirmation");
        assert!(!dest.join("Backup").exists());

        let progress = Mutex::new(Vec::new());
        let report = service
            .run_job(id, manual(true), |p| progress.lock().unwrap().push(p.done))
            .await
            .unwrap();
        assert_eq!(report.status, "success");
        assert_eq!(report.report.uploaded, 2);
        assert_eq!(
            std::fs::read_to_string(dest.join("Backup/Docs/a.txt")).unwrap(),
            "a"
        );
        assert!(!progress.lock().unwrap().is_empty());

        // Já houve sucesso: a próxima não pede confirmação e não tem nada a fazer.
        let report = service.run_job(id, manual(false), |_| {}).await.unwrap();
        assert_eq!(report.report.uploaded, 0);
        let last = service.list_jobs().unwrap()[0].last_run.clone().unwrap();
        assert_eq!(last.status, "success");
    }

    #[tokio::test]
    async fn rn43_agendada_nunca_confirma_sozinha() {
        let (_tmp, service, source, dest) = setup();
        let id = job_to_local(&service, source, dest.clone());
        let options = RunOptions {
            confirm: true,
            trigger: RunTrigger::Scheduled,
        };
        let err = service.run_job(id, options, |_| {}).await.unwrap_err();
        assert_eq!(err.code(), "needs_confirmation");
        let last = service.list_jobs().unwrap()[0].last_run.clone().unwrap();
        assert_eq!(last.status, "awaiting_confirmation");
        assert!(!dest.join("Backup").exists());
    }

    #[tokio::test]
    async fn rn90_tarefa_em_execucao_nao_roda_de_novo() {
        let (_tmp, service, source, dest) = setup();
        let id = job_to_local(&service, source, dest);
        let _held = JobLock::try_acquire(&service.paths.lock_dir, id).unwrap();
        let err = service.run_job(id, manual(true), |_| {}).await.unwrap_err();
        assert_eq!(err.code(), "busy");
        assert!(!service.cancel_run(id));
    }

    #[tokio::test]
    async fn rn90_disparo_agendado_com_a_tarefa_rodando_fica_no_historico() {
        let (_tmp, service, source, dest) = setup();
        let id = job_to_local(&service, source, dest);
        service.run_job(id, manual(true), |_| {}).await.unwrap();
        let _held = JobLock::try_acquire(&service.paths.lock_dir, id).unwrap();
        let scheduled = RunOptions {
            confirm: false,
            trigger: RunTrigger::Scheduled,
        };
        let err = service.run_job(id, scheduled, |_| {}).await.unwrap_err();
        assert_eq!(err.code(), "busy");
        let runs = service.state().unwrap().recent_runs(id, 5).unwrap();
        assert_eq!(runs[0].status, RunStatus::Discarded);
        // A última execução de verdade continua sendo a que deu certo.
        let last = service.list_jobs().unwrap()[0].last_run.clone().unwrap();
        assert_eq!(last.status, "success");
        // Um clique manual com a tarefa rodando só avisa; não vai para o histórico.
        assert!(service.run_job(id, manual(true), |_| {}).await.is_err());
        assert_eq!(
            service.state().unwrap().recent_runs(id, 5).unwrap().len(),
            2
        );
    }

    #[tokio::test]
    async fn rn56_indice_perdido_e_reconstruido_sem_reenviar() {
        let (_tmp, service, source, dest) = setup();
        let id = job_to_local(&service, source, dest);
        let first = service.run_job(id, manual(true), |_| {}).await.unwrap();
        assert_eq!(first.report.uploaded, 2);

        // Índice perdido (ex.: app reinstalado): o destino continua com tudo.
        service.state().unwrap().clear_job(id).unwrap();
        let again = service.run_job(id, manual(true), |_| {}).await.unwrap();
        assert_eq!((again.report.uploaded, again.report.updated), (0, 0));
        assert!(again.report.adopted >= 2, "{:?}", again.report);
        assert_eq!(
            service
                .state()
                .unwrap()
                .load_index(id)
                .unwrap()
                .values()
                .filter(|e| e.kind == syncly_core::domain::EntryKind::File)
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn rn40_fonte_sumida_bloqueia_e_fica_no_historico() {
        let (_tmp, service, source, dest) = setup();
        let id = job_to_local(&service, source.clone(), dest);
        std::fs::rename(&source, source.with_file_name("Movida")).unwrap();
        let err = service.run_job(id, manual(true), |_| {}).await.unwrap_err();
        assert!(err.to_string().contains("bloqueada"), "{err}");
        let last = service.list_jobs().unwrap()[0].last_run.clone().unwrap();
        assert_eq!(last.status, "aborted");
    }

    #[tokio::test]
    async fn rn05_remover_conta_desativa_tarefas() {
        let (_tmp, service, source, dest) = setup();
        let account = service.add_local_account("HD".into(), dest).unwrap();
        service
            .save_job(JobDraft {
                id: None,
                name: "Docs".into(),
                enabled: true,
                sources: vec![source],
                exclude: vec![],
                include_sensitive: false,
                sync: Some(SyncDraft {
                    account_id: account.id,
                    path: "Docs".into(),
                    delivery: Delivery::Files,
                    mode: SyncMode::Upload,
                    schedule: None,
                }),
            })
            .unwrap();
        service.remove_account(account.id).await.unwrap();
        let jobs = service.list_jobs().unwrap();
        assert!(!jobs[0].enabled);
        assert!(service.list_accounts().unwrap().is_empty());
    }

    #[test]
    fn rn94_preferencias_validadas_e_gravadas() {
        let (_tmp, service, _source, _dest) = setup();
        let mut prefs = service.preferences().unwrap();
        assert_eq!(prefs.notify, syncly_core::config::NotifyPolicy::Failures);
        prefs.notify = syncly_core::config::NotifyPolicy::Always;
        prefs.min_free_space_mib = 2048;
        prefs.max_parallel_transfers = 2;
        assert_eq!(service.save_preferences(prefs.clone()).unwrap(), prefs);
        assert_eq!(service.preferences().unwrap(), prefs);

        // Fora dos limites: recusado, nada muda.
        let bad = Preferences {
            max_parallel_transfers: 9,
            ..prefs.clone()
        };
        assert_eq!(service.save_preferences(bad).unwrap_err().code(), "invalid");
        let bad = Preferences {
            min_free_space_mib: syncly_core::config::MAX_FREE_SPACE_MIB + 1,
            ..prefs.clone()
        };
        assert_eq!(service.save_preferences(bad).unwrap_err().code(), "invalid");
        assert_eq!(service.preferences().unwrap(), prefs);
    }

    #[test]
    fn tarefa_invalida_nao_e_gravada() {
        let (_tmp, service, _source, _dest) = setup();
        let err = service
            .save_job(JobDraft {
                id: None,
                name: "".into(),
                enabled: true,
                sources: vec!["relativo".into()],
                exclude: vec![],
                include_sensitive: false,
                sync: None,
            })
            .unwrap_err();
        assert_eq!(err.code(), "invalid");
        assert!(service.list_jobs().unwrap().is_empty());
    }
}
