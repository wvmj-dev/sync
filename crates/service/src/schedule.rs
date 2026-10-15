//! Agendamentos (RN-18, RN-36, RN-80 a RN-83, RN-94): mantém os timers do systemd do
//! usuário iguais à configuração e avisa quando uma execução agendada precisa de atenção.

use std::collections::BTreeMap;
use std::path::Path;

use syncly_core::CoreError;
use syncly_core::domain::{Job, RunTrigger};
use syncly_core::fsutil::write_atomic;
use syncly_core::state::StateDb;
use syncly_core::timers::{self, HEADER, TimerState};
use uuid::Uuid;

use crate::dto::RunReport;
use crate::error::{ServiceError, ServiceResult};
use crate::service::{RunOptions, Service};

impl Service {
    /// Deixa os timers do systemd iguais ao que a configuração pede: cria, atualiza e
    /// remove só as units do próprio app. Idempotente — sem mudanças, nada é chamado.
    /// Retorna quantas tarefas ficaram com timer.
    pub fn reconcile_schedules(&self) -> ServiceResult<usize> {
        let config = self.load()?;
        let state = self.state()?;
        let dir = self.paths.systemd_unit_dir();
        let mut wanted = Vec::new();
        for job in &config.jobs {
            if let Some(sync) = &job.sync
                && timers::wants_timer(job, has_success(&state, job)?)
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

    /// Execução disparada por um timer: nunca confirma sozinha (RN-43) e avisa por
    /// notificação quando algo precisa de atenção (RN-94).
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
        let result = self.run_job(id, options, |_| {}).await;
        if let Some((title, body)) = notification(&name, &result)
            && let Err(e) = self.os.notify(&title, &body).await
        {
            tracing::warn!("notificação não enviada: {e}");
        }
        result
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
    Ok(timers::timer_state(
        job,
        has_success(state, job)?,
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

/// RN-94: o que avisar ao fim de uma execução agendada. Sucesso é silencioso, e um
/// disparo enquanto a tarefa já roda também (RN-90).
pub(crate) fn notification(
    job: &str,
    result: &ServiceResult<RunReport>,
) -> Option<(String, String)> {
    match result {
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
        Err(e) => Some((format!("{job}: falhou"), e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{JobDraft, SyncDraft};
    use crate::test_support::{FAKE_EXE, FakeOs, service_with_os};
    use std::path::PathBuf;
    use std::sync::Arc;
    use syncly_core::domain::{Delivery, Schedule, SyncMode};

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

    #[test]
    fn rn90_disparo_com_a_tarefa_rodando_nao_avisa() {
        let busy: ServiceResult<RunReport> = Err(CoreError::JobLocked.into());
        assert_eq!(notification("Docs", &busy), None);
    }
}
