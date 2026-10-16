//! Agendamentos (RN-18, RN-36, RN-80 a RN-83, RN-94): mantém os timers do systemd do
//! usuário iguais à configuração e avisa quando uma execução agendada precisa de atenção.

use std::collections::BTreeMap;
use std::path::Path;

use syncly_core::CoreError;
use syncly_core::config::NotifyPolicy;
use syncly_core::domain::{Job, RunStatus, RunTrigger};
use syncly_core::fsutil::write_atomic;
use syncly_core::state::StateDb;
use syncly_core::timers::{self, HEADER, PostponeReason, TimerState};
use uuid::Uuid;

use crate::dto::RunReport;
use crate::error::{ServiceError, ServiceResult, postponed_message};
use crate::service::{RunOptions, Service};

impl Service {
    /// Deixa os timers do systemd iguais ao que a configuração pede: cria, atualiza e
    /// remove só as units do próprio app. Idempotente — sem mudanças, nada é chamado.
    /// Retorna quantas tarefas ficaram com timer.
    pub fn reconcile_schedules(&self) -> ServiceResult<usize> {
        let config = self.load()?;
        let state = self.state()?;
        let dir = self.paths.systemd_unit_dir();
        let reconnect = state.accounts_needing_reconnect()?;
        let mut wanted = Vec::new();
        for job in &config.jobs {
            if let Some(sync) = &job.sync
                && timers::wants_timer(
                    job,
                    has_success(&state, job)?,
                    !reconnect.contains_key(&sync.destination.account_id),
                )
            {
                wanted.push((job.id, &sync.schedule));
            }
        }
        let mut desired = BTreeMap::new();
        if !wanted.is_empty() {
            let exe = self.os.exe()?;
            for (id, schedule) in &wanted {
                let units =
                    timers::render_units(*id, schedule, &exe).map_err(ServiceError::Invalid)?;
                desired.extend(units);
            }
        }
        let active = wanted.len();
        let plan = timers::plan_timers(&desired, &installed_units(&dir)?);
        if plan.is_empty() {
            return Ok(active);
        }
        for timer in &plan.stop {
            // Pode já não estar carregado; a remoção segue de qualquer forma.
            if let Err(e) = self.os.systemctl(&["disable", "--now", timer]) {
                tracing::warn!("{e}");
            }
        }
        for name in &plan.remove {
            // Só units com o cabeçalho do app chegam aqui (`plan_timers`).
            match std::fs::remove_file(dir.join(name)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    return Err(CoreError::Io {
                        path: dir.join(name),
                        source: e,
                    }
                    .into());
                }
                _ => {}
            }
        }
        if !plan.write.is_empty() && !dir.is_dir() {
            std::fs::create_dir_all(&dir).map_err(|source| CoreError::Io {
                path: dir.clone(),
                source,
            })?;
        }
        for (name, content) in &plan.write {
            write_atomic(&dir.join(name), content.as_bytes())?;
        }
        self.os.systemctl(&["daemon-reload"])?;
        for timer in &plan.start {
            self.os.systemctl(&["enable", timer])?;
            self.os.systemctl(&["restart", timer])?;
        }
        tracing::info!(
            active,
            written = plan.write.len(),
            removed = plan.remove.len(),
            "agendamentos atualizados"
        );
        Ok(active)
    }

    /// Reconcilia sem interromper quem chamou (salvar a tarefa já deu certo); um problema
    /// aparece como "não instalado" na tarefa e nos logs.
    pub(crate) fn refresh_schedules(&self) {
        if let Err(e) = self.reconcile_schedules() {
            tracing::warn!("agendamentos não atualizados: {e}");
        }
    }

    /// Execução disparada por um timer: nunca confirma sozinha (RN-43), fica adiada sem
    /// rede ou com o cofre de senhas bloqueado (RN-84) e avisa por notificação quando algo
    /// precisa de atenção (RN-94).
    pub async fn run_scheduled(&self, id: Uuid) -> ServiceResult<RunReport> {
        let name = self
            .load()?
            .job(id)
            .map(|j| j.name.clone())
            .unwrap_or_else(|| "tarefa".into());
        let options = RunOptions {
            confirm: false,
            trigger: RunTrigger::Scheduled,
        };
        let policy = self.load()?.settings.notify;
        let before = self.newest_run(id);
        let result = match self.run_job(id, options, |_| {}).await {
            // Só se nada foi registrado: um erro no meio do envio fica como a execução o gravou.
            Err(e) if e.postpone_reason().is_some() && self.newest_run(id) == before => {
                Err(self.postpone(id, &e))
            }
            other => other,
        };
        if let Some((title, body)) = notification(&name, &result, policy)
            && let Err(e) = self.os.notify(&title, &body).await
        {
            tracing::warn!("notificação não enviada: {e}");
        }
        result
    }

    /// RN-84: registra o adiamento no histórico e conta quantos já são seguidos.
    fn postpone(&self, id: Uuid, cause: &ServiceError) -> ServiceError {
        let reason = cause.postpone_reason().unwrap_or(PostponeReason::Offline);
        tracing::warn!(job = %id, "execução agendada adiada: {cause}");
        let streak = self
            .state()
            .and_then(|s| Ok(s.postponed_streak(id)?))
            .unwrap_or(0)
            .saturating_add(1);
        let message = postponed_message(reason, streak);
        self.record_unstarted(id, RunStatus::Postponed, &message);
        ServiceError::Postponed { reason, streak }
    }

    /// A execução mais recente no histórico (qualquer status).
    fn newest_run(&self, id: Uuid) -> Option<Uuid> {
        let runs = self.state().ok()?.recent_runs(id, 1).ok()?;
        runs.first().map(|r| r.id)
    }
}

/// RN-36: já houve uma execução bem-sucedida do tipo que a tarefa faz?
pub(crate) fn has_success(state: &StateDb, job: &Job) -> ServiceResult<bool> {
    let Some(sync) = &job.sync else {
        return Ok(false);
    };
    Ok(state.has_successful_run(job.id, sync.delivery.run_kind())?)
}

/// Situação do agendamento de uma tarefa, olhando a unit instalada.
pub(crate) fn timer_state(state: &StateDb, dir: &Path, job: &Job) -> ServiceResult<TimerState> {
    let unit = dir.join(format!("{}.timer", timers::unit_base(job.id)));
    let installed = std::fs::read_to_string(unit).is_ok_and(|c| c.starts_with(HEADER));
    let account_ok = job.sync.as_ref().is_none_or(|s| {
        state
            .accounts_needing_reconnect()
            .map(|m| !m.contains_key(&s.destination.account_id))
            .unwrap_or(true)
    });
    Ok(timers::timer_state(
        job,
        has_success(state, job)?,
        account_ok,
        installed,
    ))
}

/// Units do app instaladas (nome → conteúdo). Outros arquivos da pasta são ignorados.
fn installed_units(dir: &Path) -> ServiceResult<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(source) => {
            return Err(CoreError::Io {
                path: dir.to_path_buf(),
                source,
            }
            .into());
        }
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if timers::is_unit_name(&name)
            && entry.file_type().is_ok_and(|t| t.is_file())
            && let Ok(content) = std::fs::read_to_string(entry.path())
        {
            out.insert(name, content);
        }
    }
    Ok(out)
}

/// RN-94: o que avisar ao fim de uma execução agendada, conforme a preferência. Por padrão
/// só o que precisa de atenção; um disparo com a tarefa já rodando nunca avisa (RN-90).
pub(crate) fn notification(
    job: &str,
    result: &ServiceResult<RunReport>,
    policy: NotifyPolicy,
) -> Option<(String, String)> {
    if policy == NotifyPolicy::Never {
        return None;
    }
    match result {
        Ok(r) if r.status == "success" && policy == NotifyPolicy::Always => {
            Some((format!("{job}: concluída"), success_summary(r)))
        }
        Ok(r) => match r.status {
            "success" | "cancelled" => None,
            "partial_success" => Some((
                format!("{job}: concluída com falhas"),
                format!(
                    "{} item(ns) falharam — veja os detalhes no Syncly.",
                    r.failure_count
                ),
            )),
            _ => Some((
                format!("{job}: não concluiu"),
                r.report
                    .aborted
                    .clone()
                    .unwrap_or_else(|| "Veja os detalhes no Syncly.".into()),
            )),
        },
        Err(ServiceError::NeedsConfirmation(reasons)) => Some((
            format!("{job}: aguarda confirmação"),
            format!(
                "{}. Abra o Syncly e execute a tarefa uma vez manualmente.",
                reasons.join("; ")
            ),
        )),
        Err(ServiceError::Busy(_) | ServiceError::Core(CoreError::JobLocked)) => None,
        // RN-84: adiada só avisa no 3º adiamento seguido.
        Err(ServiceError::Postponed { reason, streak }) => {
            timers::postponement(*reason, *streak).notify.then(|| {
                (
                    format!("{job}: adiada {streak} vezes seguidas"),
                    sentence(&postponed_message(*reason, *streak)),
                )
            })
        }
        // RN-06, RN-85
        Err(ServiceError::NeedsReconnect(account)) => Some((
            format!("{job}: reconecte a conta"),
            format!(
                "A conta {account} precisa ser reconectada (Contas → Reconectar). Os \
                 agendamentos dela ficam pausados até lá."
            ),
        )),
        Err(e) => Some((format!("{job}: falhou"), e.to_string())),
    }
}

/// "sem conexão; …" → "Sem conexão; …."
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    let first: String = chars
        .next()
        .into_iter()
        .flat_map(char::to_uppercase)
        .collect();
    format!("{first}{}.", chars.as_str())
}

/// Uma linha sobre o que uma execução bem-sucedida fez.
fn success_summary(r: &RunReport) -> String {
    if let Some(a) = &r.archive {
        return match &a.name {
            Some(name) => format!("Versão {name} enviada."),
            None => "Nada mudou desde a última versão.".into(),
        };
    }
    if let Some(rx) = &r.receive {
        return if rx.unchanged {
            "A versão mais recente já tinha sido recebida.".into()
        } else {
            format!("{} recebida e descompactada.", rx.name)
        };
    }
    let e = &r.report;
    match e.uploaded + e.updated {
        0 => "Nada a enviar: o destino já estava igual.".into(),
        1 => "1 arquivo enviado.".into(),
        n => format!("{n} arquivos enviados."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{JobDraft, SyncDraft};
    use crate::secrets::{SecretStore, account_key};
    use crate::test_support::{
        FAKE_EXE, FakeOs, GOOGLE_CLIENT_JSON, LockedStore, service_with_os, service_with_secrets,
    };
    use std::path::PathBuf;
    use std::sync::Arc;
    use syncly_core::domain::{Account, AccountKind, Delivery, Schedule, SyncMode};

    struct Setup {
        _tmp: tempfile::TempDir,
        os: Arc<FakeOs>,
        service: Service,
        source: PathBuf,
        account: Uuid,
        units: PathBuf,
    }

    fn setup() -> Setup {
        let tmp = tempfile::tempdir().unwrap();
        let os = Arc::new(FakeOs::default());
        let service = service_with_os(&tmp.path().join("app"), "http://127.0.0.1:9", os.clone());
        let source = tmp.path().join("Docs");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("a.txt"), "a").unwrap();
        let hd = tmp.path().join("hd");
        std::fs::create_dir_all(&hd).unwrap();
        let account = service.add_local_account("HD".into(), hd).unwrap().id;
        let units = service.paths.systemd_unit_dir();
        Setup {
            _tmp: tmp,
            os,
            service,
            source,
            account,
            units,
        }
    }

    fn daily() -> Schedule {
        Schedule::Daily {
            at: "02:30".to_string().try_into().unwrap(),
        }
    }

    fn save(s: &Setup, id: Option<Uuid>, enabled: bool, schedule: Schedule) -> ServiceResult<Uuid> {
        s.service
            .save_job(JobDraft {
                id,
                name: "Docs".into(),
                enabled,
                sources: vec![s.source.clone()],
                exclude: vec![],
                include_sensitive: false,
                sync: Some(SyncDraft {
                    account_id: s.account,
                    path: "Docs".into(),
                    delivery: Delivery::Files,
                    mode: SyncMode::Upload,
                    schedule: Some(schedule),
                }),
            })
            .map(|j| j.id)
    }

    fn state(s: &Setup) -> &'static str {
        s.service.list_jobs().unwrap()[0]
            .sync
            .as_ref()
            .unwrap()
            .schedule_state
    }

    fn manual() -> RunOptions {
        RunOptions {
            confirm: true,
            trigger: RunTrigger::Manual,
        }
    }

    #[tokio::test]
    async fn rn36_rn18_timer_so_depois_da_primeira_execucao_manual() {
        let s = setup();
        let id = save(&s, None, true, daily()).unwrap();
        // RN-36: ainda sem execução bem-sucedida — nada instalado, nada chamado.
        assert_eq!(state(&s), "waiting_first_run");
        assert!(!s.units.exists());
        assert!(s.os.calls().is_empty());

        s.service.run_job(id, manual(), |_| {}).await.unwrap();
        let timer_name = format!("syncly-{id}.timer");
        let timer = std::fs::read_to_string(s.units.join(&timer_name)).unwrap();
        let unit = std::fs::read_to_string(s.units.join(format!("syncly-{id}.service"))).unwrap();
        assert!(timer.contains("OnCalendar=*-*-* 02:30:00"));
        assert!(unit.contains(&format!("ExecStart=\"{FAKE_EXE}\" run {id} --scheduled")));
        assert_eq!(
            s.os.calls(),
            [
                "systemctl daemon-reload".to_string(),
                format!("systemctl enable {timer_name}"),
                format!("systemctl restart {timer_name}"),
            ]
        );
        assert_eq!(state(&s), "active");

        // Idempotente: nada mudou, nada é chamado.
        let before = s.os.calls().len();
        s.service.reconcile_schedules().unwrap();
        assert_eq!(s.os.calls().len(), before);

        // Novo horário: o timer é regravado e reiniciado.
        save(&s, Some(id), true, Schedule::EveryHours { hours: 6 }).unwrap();
        let timer = std::fs::read_to_string(s.units.join(&timer_name)).unwrap();
        assert!(timer.contains("OnCalendar=*-*-* 00/6:00:00"));
        assert!(
            s.os.calls()
                .ends_with(&[format!("systemctl restart {timer_name}")])
        );

        // Uma unit criada à mão com o nosso prefixo nunca é tocada.
        let foreign = s.units.join(format!("syncly-{}.timer", Uuid::new_v4()));
        std::fs::write(&foreign, "[Timer]\nOnCalendar=daily\n").unwrap();

        // RN-18: desativada → timer desligado e removido.
        save(&s, Some(id), false, daily()).unwrap();
        assert!(
            s.os.calls()
                .contains(&format!("systemctl disable --now {timer_name}"))
        );
        assert!(!s.units.join(&timer_name).exists());
        assert_eq!(state(&s), "disabled");
        assert!(foreign.exists());

        // Excluir a tarefa também não deixa nada para trás.
        save(&s, Some(id), true, daily()).unwrap();
        assert!(s.units.join(&timer_name).exists());
        s.service.delete_job(id).unwrap();
        assert!(!s.units.join(&timer_name).exists());
        assert!(foreign.exists());
    }

    #[test]
    fn rn82_expressao_avancada_passa_pelo_systemd() {
        let s = setup();
        let bad = Schedule::Calendar {
            expr: "toda invalida".into(),
        };
        let err = save(&s, None, true, bad).unwrap_err();
        assert!(err.to_string().contains("inválida"), "{err}");
        assert!(s.service.list_jobs().unwrap().is_empty());

        let ok = Schedule::Calendar {
            expr: "Mon..Fri *-*-* 09:00:00".into(),
        };
        save(&s, None, true, ok).unwrap();
        assert!(
            s.os.calls()
                .contains(&"verify Mon..Fri *-*-* 09:00:00".to_string())
        );
    }

    #[tokio::test]
    async fn rn43_rn94_agendada_avisa_quando_precisa_de_atencao() {
        let s = setup();
        let id = save(&s, None, true, daily()).unwrap();
        let notices = || -> Vec<String> {
            s.os.calls()
                .into_iter()
                .filter(|c| c.starts_with("notify"))
                .collect()
        };

        // RN-43: nunca confirma sozinha — e avisa.
        let err = s.service.run_scheduled(id).await.unwrap_err();
        assert_eq!(err.code(), "needs_confirmation");
        assert_eq!(notices().len(), 1);
        assert!(
            notices()[0].contains("aguarda confirmação"),
            "{:?}",
            notices()
        );

        // Sucesso é silencioso.
        s.service.run_job(id, manual(), |_| {}).await.unwrap();
        std::fs::write(s.source.join("b.txt"), "b").unwrap();
        let report = s.service.run_scheduled(id).await.unwrap();
        assert_eq!(report.status, "success");
        assert_eq!(notices().len(), 1);

        // Fonte sumiu (RN-40): bloqueada, com aviso.
        std::fs::rename(&s.source, s.source.with_file_name("Movida")).unwrap();
        assert!(s.service.run_scheduled(id).await.is_err());
        assert_eq!(notices().len(), 2);
        assert!(notices()[1].contains("falhou"), "{:?}", notices());
    }

    /// Conta do Google e uma tarefa agendada que envia `source` para ela.
    fn google_job(svc: &Service, source: PathBuf, secrets: &dyn SecretStore) -> Uuid {
        std::fs::write(svc.paths.google_client_file(), GOOGLE_CLIENT_JSON).unwrap();
        let account = Uuid::new_v4();
        svc.mutate(|c| {
            c.accounts.push(Account {
                id: account,
                label: "eu@gmail.com".into(),
                kind: AccountKind::GoogleDrive {
                    email: Some("eu@gmail.com".into()),
                    root_folder_id: Some("ROOT1".into()),
                },
            });
            Ok(())
        })
        .unwrap();
        let _ = secrets.set(&account_key(account), "1//r");
        std::fs::create_dir_all(&source).unwrap();
        svc.save_job(JobDraft {
            id: None,
            name: "Docs".into(),
            enabled: true,
            sources: vec![source],
            exclude: vec![],
            include_sensitive: false,
            sync: Some(SyncDraft {
                account_id: account,
                path: "Docs".into(),
                delivery: Delivery::Files,
                mode: SyncMode::Upload,
                schedule: Some(daily()),
            }),
        })
        .unwrap()
        .id
    }

    /// Endereço onde ninguém atende — o "sem rede" dos testes.
    fn offline_url() -> String {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        format!("http://127.0.0.1:{port}")
    }

    // Relógio do tokio pausado: as esperas entre as tentativas de rede (RN-96) passam na hora.
    #[tokio::test(start_paused = true)]
    async fn rn84_sem_rede_fica_adiada_tenta_de_novo_e_avisa_no_terceiro() {
        let tmp = tempfile::tempdir().unwrap();
        let os = Arc::new(FakeOs::default());
        let secrets = Arc::new(crate::secrets::MemoryStore::default());
        let svc = service_with_secrets(
            &tmp.path().join("app"),
            &offline_url(),
            secrets.clone(),
            os.clone(),
        );
        let id = google_job(&svc, tmp.path().join("Docs"), secrets.as_ref());
        let notices = || -> Vec<String> {
            os.calls()
                .into_iter()
                .filter(|c| c.starts_with("notify"))
                .collect()
        };

        for expected in 1..=4 {
            let err = svc.run_scheduled(id).await.unwrap_err();
            assert!(
                matches!(err, ServiceError::Postponed { reason: PostponeReason::Offline, streak }
                    if streak == expected),
                "{err:?}"
            );
            assert_eq!(err.code(), "postponed");
            // O processo sai com 75 e o systemd tenta de novo em 15 min.
            assert!(err.retry_later());
        }
        // Avisa uma vez, no 3º adiamento seguido.
        assert_eq!(
            notices(),
            [
                "notify Docs: adiada 3 vezes seguidas | Sem conexão com a internet; nova \
              tentativa em 15 min."
            ]
        );
        let state = svc.state().unwrap();
        let last = state.last_run(id).unwrap().unwrap();
        assert_eq!(last.status, RunStatus::Postponed);
        assert_eq!(last.trigger, RunTrigger::Scheduled);
        assert_eq!(
            last.error.as_deref(),
            Some("sem conexão com a internet; nova tentativa em 15 min")
        );
        // Sem rede não é problema de login: a conta não fica marcada (RN-06).
        assert!(
            svc.list_accounts().unwrap()[0]
                .needs_reconnect_since
                .is_none()
        );

        // Execução manual sem rede: só o erro, sem adiamento no histórico.
        let err = svc.run_job(id, manual(), |_| {}).await.unwrap_err();
        assert_eq!(err.code(), "provider");
        assert_eq!(state.postponed_streak(id).unwrap(), 4);
    }

    #[tokio::test]
    async fn rn84_cofre_bloqueado_fica_adiada_sem_nova_tentativa() {
        let tmp = tempfile::tempdir().unwrap();
        let os = Arc::new(FakeOs::default());
        let svc = service_with_secrets(
            &tmp.path().join("app"),
            &offline_url(),
            Arc::new(LockedStore),
            os,
        );
        let id = google_job(&svc, tmp.path().join("Docs"), &LockedStore);

        let err = svc.run_scheduled(id).await.unwrap_err();
        assert!(
            matches!(
                err,
                ServiceError::Postponed {
                    reason: PostponeReason::KeyringLocked,
                    streak: 1
                }
            ),
            "{err:?}"
        );
        // Tentar de novo sozinho abriria o pedido de senha a cada 15 min: espera o horário.
        assert!(!err.retry_later());
        let last = svc.state().unwrap().last_run(id).unwrap().unwrap();
        assert_eq!(
            last.error.as_deref(),
            Some(
                "o cofre de senhas do sistema está bloqueado; nova tentativa no próximo horário agendado"
            )
        );
    }

    #[test]
    fn rn84_adiada_avisa_so_no_terceiro_seguido() {
        let postponed = |streak| -> ServiceResult<RunReport> {
            Err(ServiceError::Postponed {
                reason: PostponeReason::KeyringLocked,
                streak,
            })
        };
        assert_eq!(
            notification("Docs", &postponed(2), NotifyPolicy::Always),
            None
        );
        assert_eq!(
            notification("Docs", &postponed(3), NotifyPolicy::Never),
            None
        );
        let (title, body) = notification("Docs", &postponed(3), NotifyPolicy::Failures).unwrap();
        assert_eq!(title, "Docs: adiada 3 vezes seguidas");
        assert_eq!(
            body,
            "O cofre de senhas do sistema está bloqueado; nova tentativa no próximo horário \
             agendado."
        );
    }

    #[test]
    fn rn90_disparo_com_a_tarefa_rodando_nao_avisa() {
        let busy: ServiceResult<RunReport> = Err(CoreError::JobLocked.into());
        for policy in [
            NotifyPolicy::Always,
            NotifyPolicy::Failures,
            NotifyPolicy::Never,
        ] {
            assert_eq!(notification("Docs", &busy, policy), None);
        }
    }

    #[test]
    fn rn94_preferencia_decide_o_que_avisar() {
        let ok: ServiceResult<RunReport> = Ok(RunReport::new(
            syncly_core::domain::RunStatus::Success,
            syncly_core::sync::executor::ExecutionReport {
                uploaded: 3,
                ..Default::default()
            },
            10,
        ));
        let failed: ServiceResult<RunReport> = Err(ServiceError::Invalid("sem rede".into()));
        // Padrão: só problemas.
        assert_eq!(notification("Docs", &ok, NotifyPolicy::Failures), None);
        assert!(notification("Docs", &failed, NotifyPolicy::Failures).is_some());
        // Sempre: o sucesso também avisa.
        let (title, body) = notification("Docs", &ok, NotifyPolicy::Always).unwrap();
        assert_eq!(
            (title.as_str(), body.as_str()),
            ("Docs: concluída", "3 arquivos enviados.")
        );
        // Nunca.
        assert_eq!(notification("Docs", &failed, NotifyPolicy::Never), None);
    }
}
