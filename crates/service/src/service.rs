use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use syncly_core::CoreError;
use syncly_core::config::AppConfig;
use syncly_core::domain::{
    Account, AccountKind, Destination, GuardLimits, Job, RelPath, RunKind, Schedule, SkipReason,
    SyncSettings, ValidationContext,
};
use syncly_core::filters::Filter;
use syncly_core::paths::AppPaths;
use syncly_core::provider::StorageProvider;
use syncly_core::providers::LocalProvider;
use syncly_core::state::StateDb;
use syncly_core::sync::guard::{self, GuardInput};
use syncly_core::sync::planner::{self, PlanInput};
use syncly_core::sync::remote_tree::load_remote_tree;
use syncly_core::sync::scanner::{self, ScanRequest};
use syncly_gdrive::auth::{ClientConfigError, load_client};
use uuid::Uuid;

use crate::dto::{
    AccountSummary, ActionView, AppInfo, JobDraft, JobSummary, MAX_PREVIEW_ACTIONS,
    MAX_PREVIEW_WARNINGS, PlanPreview, SyncSummary,
};
use crate::error::{ServiceError, ServiceResult};

/// Ponto de entrada dos casos de uso. A configuração é relida do disco a cada
/// chamada (arquivo pequeno), então GUI e CLI sempre enxergam a mesma versão.
pub struct Service {
    paths: AppPaths,
    write_lock: Mutex<()>,
}

impl Service {
    pub fn open(paths: AppPaths) -> ServiceResult<Self> {
        paths.ensure()?;
        Ok(Self {
            paths,
            write_lock: Mutex::new(()),
        })
    }

    pub fn open_default() -> ServiceResult<Self> {
        Self::open(AppPaths::discover()?)
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    fn load(&self) -> ServiceResult<AppConfig> {
        Ok(AppConfig::load(&self.paths.config_file())?)
    }

    /// Lê → altera → valida → grava atomicamente. Uma alteração inválida nunca chega ao disco.
    fn mutate<T>(&self, f: impl FnOnce(&mut AppConfig) -> ServiceResult<T>) -> ServiceResult<T> {
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

    fn state(&self) -> ServiceResult<StateDb> {
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

    // ---- Contas ----

    pub fn list_accounts(&self) -> ServiceResult<Vec<AccountSummary>> {
        let config = self.load()?;
        Ok(config
            .accounts
            .iter()
            .map(|a| account_summary(&config, a))
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
        Ok(account_summary(&self.load()?, &saved))
    }

    /// RN-05: as tarefas que usavam a conta são desativadas, não apagadas.
    /// Fase 1: contas Google revogam o token antes (SEG-06).
    pub fn remove_account(&self, id: Uuid) -> ServiceResult<()> {
        self.mutate(|config| {
            let before = config.accounts.len();
            config.accounts.retain(|a| a.id != id);
            if config.accounts.len() == before {
                return Err(ServiceError::NotFound("conta"));
            }
            for job in &mut config.jobs {
                let uses_sync = job
                    .sync
                    .as_ref()
                    .is_some_and(|s| s.destination.account_id == id);
                let uses_backup = job
                    .backup
                    .as_ref()
                    .is_some_and(|b| b.remote_account_id == Some(id));
                if uses_sync || uses_backup {
                    job.enabled = false;
                }
            }
            Ok(())
        })
    }

    // ---- Tarefas ----

    pub fn list_jobs(&self) -> ServiceResult<Vec<JobSummary>> {
        let config = self.load()?;
        let state = self.state()?;
        config
            .jobs
            .iter()
            .map(|j| job_summary(&config, &state, j))
            .collect()
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
                s.mode,
            )),
            None => None,
        };
        let id = self.mutate(|config| {
            let id = draft.id.unwrap_or_else(Uuid::new_v4);
            let existing = config.jobs.iter().position(|j| j.id == id);
            if draft.id.is_some() && existing.is_none() {
                return Err(ServiceError::NotFound("tarefa"));
            }
            let previous = existing.map(|i| config.jobs[i].clone());
            let previous_sync = previous.as_ref().and_then(|j| j.sync.clone());
            let job = Job {
                id,
                name: draft.name.trim().to_string(),
                enabled: draft.enabled,
                sources: draft.sources.clone(),
                exclude: clean_patterns(&draft.exclude),
                include_sensitive: draft.include_sensitive,
                sync: sync.map(|(account_id, path, mode)| SyncSettings {
                    destination: Destination { account_id, path },
                    mode,
                    schedule: previous_sync
                        .as_ref()
                        .map_or(Schedule::Manual, |s| s.schedule.clone()),
                    guard: previous_sync
                        .as_ref()
                        .map_or(GuardLimits::default(), |s| s.guard),
                }),
                backup: previous.and_then(|j| j.backup),
            };
            match existing {
                Some(i) => config.jobs[i] = job,
                None => config.jobs.push(job),
            }
            Ok(id)
        })?;
        let config = self.load()?;
        let job = config.job(id).ok_or(ServiceError::NotFound("tarefa"))?;
        job_summary(&config, &self.state()?, job)
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
        Ok(())
    }

    // ---- Sincronização ----

    /// Prévia: escaneia, lista o destino, planeja e avalia as guardas — sem alterar nada.
    pub async fn preview_job(&self, id: Uuid) -> ServiceResult<PlanPreview> {
        let started = Instant::now();
        let config = self.load()?;
        let job = config
            .job(id)
            .cloned()
            .ok_or(ServiceError::NotFound("tarefa"))?;
        // O config.toml pode ter sido editado à mão: valida antes de tocar em qualquer arquivo.
        let app_dirs = self.paths.app_dirs();
        let issues = job.validate(&ValidationContext {
            app_dirs: &app_dirs,
        });
        if !issues.is_empty() {
            return Err(CoreError::Invalid(issues).into());
        }
        let sync = job.sync.clone().ok_or_else(|| {
            ServiceError::Invalid("esta tarefa não tem sincronização configurada".into())
        })?;
        let account = config
            .account(sync.destination.account_id)
            .cloned()
            .ok_or(ServiceError::NotFound("conta de destino"))?;
        let provider = provider_for(&account)?;

        let state = self.state()?;
        let index = state.load_index(job.id)?;
        let first_run = !state.has_successful_run(job.id, RunKind::Sync)?;
        drop(state);

        let request = ScanRequest {
            sources: job.sources.clone(),
            filter: Filter::new(&job.exclude, job.include_sensitive)
                .map_err(ServiceError::Invalid)?,
            skip_dirs: self.paths.app_dirs(),
            hash_kind: provider.hash_kind(),
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

        let warnings = scan
            .warnings
            .iter()
            .map(|w| w.describe())
            .chain(
                remote
                    .invalid_names
                    .iter()
                    .map(|n| format!("nome remoto ignorado: {n}")),
            )
            .take(MAX_PREVIEW_WARNINGS)
            .collect();

        tracing::info!(
            job = %job.id,
            actions = plan.actions.len(),
            hashed = scan.hashed_files,
            "prévia calculada"
        );
        Ok(PlanPreview {
            job_id: job.id,
            mode: sync.mode,
            total_actions: plan.actions.len(),
            actions: plan
                .actions
                .iter()
                .take(MAX_PREVIEW_ACTIONS)
                .map(ActionView::from)
                .collect(),
            stats: plan.stats,
            verdict,
            warnings,
            hashed_files: scan.hashed_files,
            duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            first_run,
        })
    }
}

/// Fábrica de provedores: o único lugar que conhece todos eles (ARQUITETURA.md §2).
fn provider_for(account: &Account) -> ServiceResult<Box<dyn StorageProvider>> {
    match &account.kind {
        AccountKind::Local { root } => Ok(Box::new(LocalProvider::new(root.clone()))),
        AccountKind::GoogleDrive { .. } => Err(ServiceError::Unavailable(
            "o provedor Google Drive chega na fase 1 (ver docs/ARQUITETURA.md)".into(),
        )),
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

fn account_summary(config: &AppConfig, account: &Account) -> AccountSummary {
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
                || j.backup
                    .as_ref()
                    .is_some_and(|b| b.remote_account_id == Some(account.id))
        })
        .count();
    AccountSummary {
        id: account.id,
        label: account.label.clone(),
        provider: account.kind.provider_id(),
        detail,
        job_count,
    }
}

fn job_summary(config: &AppConfig, state: &StateDb, job: &Job) -> ServiceResult<JobSummary> {
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
            mode: s.mode,
            schedule: s.schedule.label(),
        }),
        backup_enabled: job.backup.is_some(),
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
        let service = Service::open(AppPaths::in_dir(&tmp.path().join("app"))).unwrap();
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
                    mode: SyncMode::Mirror,
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

    #[test]
    fn rn05_remover_conta_desativa_tarefas() {
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
                    mode: SyncMode::Upload,
                }),
            })
            .unwrap();
        service.remove_account(account.id).unwrap();
        let jobs = service.list_jobs().unwrap();
        assert!(!jobs[0].enabled);
        assert!(service.list_accounts().unwrap().is_empty());
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
