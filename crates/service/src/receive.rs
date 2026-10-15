//! Tarefas que recebem (RN-107): baixam a versão mais recente de uma pasta do destino e a
//! descompactam numa pasta nova no computador. Só enxergam o que o Syncly enviou
//! (escopo `drive.file`, SEG-03).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use chrono::Utc;
use syncly_core::CoreError;
use syncly_core::archive::{ArchiveFormat, latest_version};
use syncly_core::domain::rel_path::validate_name;
use syncly_core::domain::{PlanStats, RunKind, RunStatus};
use syncly_core::provider::{RemoteEntry, StorageProvider};
use syncly_core::state::ReceivedRecord;
use syncly_core::sync::executor::{ExecutionReport, Progress};
use syncly_core::sync::guard::{self, GuardVerdict, ReceiveGuardInput};
use syncly_core::sync::remote_tree::resolve_path;
use uuid::Uuid;

use crate::archive::ensure_space;
use crate::dto::{PlanPreview, ReceivePreview, ReceiveRun, RunReport};
use crate::error::{ServiceError, ServiceResult};
use crate::restore::{RemoteFile, Target, fetch_and_extract};
use crate::service::{Loaded, RunOptions, Service, check_verdict, elapsed_ms};

/// Parâmetros de uma execução de recebimento, vindos de `run_job`.
pub(crate) struct ReceiveExec<'a> {
    pub options: RunOptions,
    pub cancel: &'a Arc<AtomicBool>,
    pub progress: &'a (dyn Fn(&Progress) + Send + Sync),
    pub started: Instant,
}

/// O que a prévia e a execução precisam, calculado sem alterar nada.
struct Prepared {
    loaded: Loaded,
    target: PathBuf,
    provider: Box<dyn StorageProvider>,
    /// Versão mais recente na pasta de origem.
    latest: Option<RemoteEntry>,
    last: Option<ReceivedRecord>,
    already_received: bool,
    verdict: GuardVerdict,
    first_run: bool,
}

impl Service {
    async fn prepare_receive(&self, loaded: Loaded, target: PathBuf) -> ServiceResult<Prepared> {
        let provider = self.provider_for(&loaded.account).await?;
        let state = self.state()?;
        let last = state.last_received(loaded.job.id)?;
        let first_run = !state.has_successful_run(loaded.job.id, RunKind::Receive)?;
        drop(state);

        let origin = &loaded.sync.destination.path;
        let latest = match resolve_path(provider.as_ref(), origin, false).await? {
            None => None,
            Some(folder) => {
                // Sem hash: no provedor local, não lê o conteúdo de cada versão.
                let entries = provider
                    .list_children_meta(&folder.id)
                    .await
                    .map_err(CoreError::from)?;
                latest_version(&entries)
                    .map_err(ServiceError::Invalid)?
                    .cloned()
            }
        };
        let already_received = matches!((&last, &latest), (Some(r), Some(e)) if r.is_same(e));
        let verdict = guard::evaluate_receive(&ReceiveGuardInput {
            target: &target,
            target_exists: target.is_dir(),
            latest_found: latest.is_some(),
            origin: origin.as_str(),
            already_received,
            first_run,
        });
        Ok(Prepared {
            loaded,
            target,
            provider,
            latest,
            last,
            already_received,
            verdict,
            first_run,
        })
    }

    pub(crate) async fn preview_receive(
        &self,
        loaded: Loaded,
        target: PathBuf,
        started: Instant,
    ) -> ServiceResult<PlanPreview> {
        let p = self.prepare_receive(loaded, target).await?;
        Ok(PlanPreview {
            job_id: p.loaded.job.id,
            mode: p.loaded.sync.mode,
            stats: PlanStats::default(),
            verdict: p.verdict,
            actions: Vec::new(),
            total_actions: 0,
            warnings: Vec::new(),
            hashed_files: 0,
            duration_ms: elapsed_ms(started),
            first_run: p.first_run,
            archive: None,
            receive: Some(ReceivePreview {
                name: p.latest.as_ref().map(|e| e.name.clone()),
                size: p.latest.as_ref().and_then(|e| e.size),
                modified_at: p.latest.as_ref().and_then(|e| e.modified_at),
                already_received: p.already_received,
                target: p.target.display().to_string(),
            }),
        })
    }

    /// Baixa a versão mais recente, confere e descompacta numa pasta nova (RN-107).
    pub(crate) async fn run_receive(
        &self,
        loaded: Loaded,
        target: PathBuf,
        exec: ReceiveExec<'_>,
    ) -> ServiceResult<RunReport> {
        let id = loaded.job.id;
        let trigger = exec.options.trigger;
        let p = self.prepare_receive(loaded, target).await?;
        let state = self.state()?;
        check_verdict(&state, id, RunKind::Receive, &p.verdict, exec.options)?;
        let latest = p
            .latest
            .clone()
            .ok_or_else(|| ServiceError::Internal("versão sumiu após a guarda".into()))?;

        if p.already_received {
            let run = state.start_run(id, RunKind::Receive, trigger)?;
            state.finish_run(run, RunStatus::Success, None, None)?;
            tracing::info!(job = %id, "versão mais recente já recebida");
            let mut out = RunReport::new(
                RunStatus::Success,
                ExecutionReport::default(),
                elapsed_ms(exec.started),
            );
            out.receive = Some(ReceiveRun {
                name: latest.name,
                dest: p.last.map(|r| r.dest).unwrap_or_default(),
                files: 0,
                bytes: 0,
                skipped_count: 0,
                unchanged: true,
            });
            return Ok(out);
        }

        let size = latest.size.unwrap_or(0);
        let staging = self.staging_dir(&id.to_string())?;
        // RN-67: o download cabe no disco sem passar da reserva.
        if let Err(e) = ensure_space(&staging, size, "o disco do computador") {
            let run = state.start_run(id, RunKind::Receive, trigger)?;
            state.finish_run(run, RunStatus::Aborted, None, Some(&e.to_string()))?;
            return Err(e);
        }
        let (format, stem) = ArchiveFormat::detect(&latest.name)
            .map(|(f, s)| (f, s.to_string()))
            .ok_or_else(|| ServiceError::Internal("versão sem formato conhecido".into()))?;
        let stem = if validate_name(&stem).is_ok() {
            stem
        } else {
            "recebido".to_string()
        };

        let run_id = state.start_run(id, RunKind::Receive, trigger)?;
        tracing::info!(job = %id, "recebimento iniciado");
        let file = RemoteFile {
            id: &latest.id,
            name: &latest.name,
            size,
            // O provedor já confere o checksum do download (RN-106).
            expected_hash: None,
        };
        // RN-57, RN-107: sempre numa pasta nova, nunca por cima.
        let target = Target {
            dir: &p.target,
            stem: &stem,
            format,
        };
        let result = fetch_and_extract(
            p.provider.as_ref(),
            &file,
            &staging,
            &target,
            exec.cancel,
            exec.progress,
        )
        .await;
        let report = match result {
            Ok(report) => report,
            Err((status, e)) => {
                state.finish_run(run_id, status, None, Some(&e.to_string()))?;
                tracing::info!(job = %id, status = status.as_str(), "recebimento interrompido");
                if status == RunStatus::Cancelled {
                    let report = ExecutionReport {
                        cancelled: true,
                        ..ExecutionReport::default()
                    };
                    return Ok(RunReport::new(status, report, elapsed_ms(exec.started)));
                }
                return Err(e);
            }
        };

        let dest = report.dest.display().to_string();
        state.insert_received(&ReceivedRecord {
            id: Uuid::new_v4(),
            job_id: id,
            remote_id: latest.id.clone(),
            name: latest.name.clone(),
            size,
            hash: latest.hash.clone(),
            dest: dest.clone(),
            received_at: Utc::now(),
        })?;
        // RN-66: entradas ignoradas por segurança tornam o recebimento parcial.
        let status = if report.skipped.is_empty() {
            RunStatus::Success
        } else {
            RunStatus::PartialSuccess
        };
        let stats = serde_json::to_value(&report).ok();
        state.finish_run(run_id, status, stats.as_ref(), None)?;
        tracing::info!(job = %id, files = report.files, status = status.as_str(), "recebimento concluído");
        let mut out = RunReport::new(status, ExecutionReport::default(), elapsed_ms(exec.started));
        out.receive = Some(ReceiveRun {
            name: latest.name,
            dest,
            files: report.files,
            bytes: report.bytes,
            skipped_count: report.skipped.len(),
            unchanged: false,
        });
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use crate::delivery::tests::{Fixture, fixture, names, options};
    use crate::dto::{JobDraft, SyncDraft};
    use std::path::{Path, PathBuf};
    use syncly_core::domain::{Delivery, Retention, SyncMode};
    use syncly_core::sync::guard::GuardVerdict;
    use uuid::Uuid;

    fn receiver(f: &Fixture, origin: &str, target: &Path) -> Uuid {
        let account = f.service.list_accounts().unwrap().remove(0);
        f.service
            .save_job(JobDraft {
                id: None,
                name: "Receber".into(),
                enabled: true,
                sources: vec![],
                exclude: vec![],
                include_sensitive: false,
                sync: Some(SyncDraft {
                    account_id: account.id,
                    // RN-17: pode ser a mesma pasta da tarefa que envia — só lê.
                    path: origin.into(),
                    delivery: Delivery::Receive {
                        target: target.to_path_buf(),
                    },
                    mode: SyncMode::Upload,
                    schedule: None,
                }),
            })
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn rn107_recebe_a_versao_mais_recente_numa_pasta_nova() {
        let f = fixture(Retention::default());
        f.service
            .run_job(f.job, options(true), |_| {})
            .await
            .unwrap();
        let target = f._tmp.path().join("Recebidos");
        std::fs::create_dir_all(&target).unwrap();
        let rx = receiver(&f, "Backups", &target);
        let sent = names(&f.backups).remove(0);

        let preview = f.service.preview_job(rx).await.unwrap();
        let planned = preview.receive.unwrap();
        assert_eq!(planned.name.as_deref(), Some(sent.as_str()));
        assert!(!planned.already_received);
        // RN-36: primeira vez pede confirmação; a prévia não cria nada.
        assert!(matches!(
            preview.verdict,
            GuardVerdict::NeedsConfirmation(_)
        ));
        assert!(names(&target).is_empty());
        let err = f
            .service
            .run_job(rx, options(false), |_| {})
            .await
            .unwrap_err();
        assert_eq!(err.code(), "needs_confirmation");

        let report = f.service.run_job(rx, options(true), |_| {}).await.unwrap();
        assert_eq!(report.status, "success");
        let got = report.receive.unwrap();
        let dest = PathBuf::from(&got.dest);
        assert_eq!(dest, target.join(sent.trim_end_matches(".zip")));
        assert_eq!(
            std::fs::read_to_string(dest.join("level.dat")).unwrap(),
            "nivel"
        );

        // Mesma versão: nada a fazer, nenhuma pasta nova.
        let report = f.service.run_job(rx, options(false), |_| {}).await.unwrap();
        assert!(report.receive.unwrap().unchanged);
        assert_eq!(names(&target).len(), 1);

        // Versão nova enviada → recebida numa pasta nova; a anterior fica intacta.
        std::fs::write(f.source.join("level.dat"), "nivel 2").unwrap();
        f.service
            .run_job(f.job, options(false), |_| {})
            .await
            .unwrap();
        let report = f.service.run_job(rx, options(false), |_| {}).await.unwrap();
        let got = report.receive.unwrap();
        assert!(!got.unchanged);
        assert_eq!(names(&target).len(), 2);
        assert_eq!(
            std::fs::read_to_string(PathBuf::from(got.dest).join("level.dat")).unwrap(),
            "nivel 2"
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("level.dat")).unwrap(),
            "nivel"
        );
        let jobs = f.service.list_jobs().unwrap();
        let last = jobs
            .iter()
            .find(|j| j.id == rx)
            .unwrap()
            .last_run
            .clone()
            .unwrap();
        assert_eq!((last.kind, last.status), ("receive", "success"));
    }

    #[tokio::test]
    async fn rn107_sem_versao_ou_sem_pasta_local_bloqueia() {
        let f = fixture(Retention::default());
        let target = f._tmp.path().join("Recebidos");
        std::fs::create_dir_all(&target).unwrap();
        // Nada enviado ainda: nenhuma versão.
        let rx = receiver(&f, "Backups", &target);
        let preview = f.service.preview_job(rx).await.unwrap();
        assert!(matches!(preview.verdict, GuardVerdict::Abort(_)));
        assert_eq!(preview.receive.unwrap().name, None);

        // RN-40: pasta do computador sumiu (disco desmontado?).
        f.service
            .run_job(f.job, options(true), |_| {})
            .await
            .unwrap();
        std::fs::remove_dir(&target).unwrap();
        let err = f
            .service
            .run_job(rx, options(true), |_| {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("não foi encontrada"), "{err}");
        assert!(!target.exists());
    }

    #[test]
    fn rn110_tarefa_que_recebe_nao_tem_fontes() {
        let f = fixture(Retention::default());
        let account = f.service.list_accounts().unwrap().remove(0);
        let draft = |sources: Vec<PathBuf>, target: &str| JobDraft {
            id: None,
            name: "Receber".into(),
            enabled: true,
            sources,
            exclude: vec![],
            include_sensitive: false,
            sync: Some(SyncDraft {
                account_id: account.id,
                path: "Backups".into(),
                delivery: Delivery::Receive {
                    target: target.into(),
                },
                mode: SyncMode::Upload,
                schedule: None,
            }),
        };
        let err = f
            .service
            .save_job(draft(vec![f.source.clone()], "/tmp/x"))
            .unwrap_err();
        assert!(err.to_string().contains("não tem fontes"), "{err}");
        let err = f.service.save_job(draft(vec![], "relativo")).unwrap_err();
        assert!(err.to_string().contains("absoluto"), "{err}");
        let err = f.service.save_job(draft(vec![], "/proc/x")).unwrap_err();
        assert!(err.to_string().contains("sistema"), "{err}");
    }
}
