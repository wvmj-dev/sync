//! Tarefas que enviam compactado (RN-50 a RN-59): a cada execução, as fontes viram um
//! único `.zip`/`.tar.*` com data no nome, enviado ao destino. Versões além da retenção
//! vão para a lixeira — só as que o próprio app registrou (RN-55).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use chrono::{Local, NaiveDateTime, Utc};
use syncly_core::archive::{
    self, ArchiveFormat, ArchiveProgress, CreateReport, CreateRequest, DISK_RESERVE_BYTES,
    fingerprint, version_name,
};
use syncly_core::backup::retention::plan_retention;
use syncly_core::domain::{EntryKind, PlanStats, Retention, RunKind, RunStatus};
use syncly_core::filters::Filter;
use syncly_core::fsutil::ensure_private_dir;
use syncly_core::provider::{ProviderError, StorageProvider};
use syncly_core::state::{ArchiveRecord, StateDb};
use syncly_core::sync::executor::{ExecutionReport, Failure, Progress};
use syncly_core::sync::guard::{self, ArchiveGuardInput, GuardVerdict};
use syncly_core::sync::remote_tree::resolve_path;
use syncly_core::sync::scanner::{self, ScanRequest, ScanResult};
use syncly_core::{CoreError, CoreResult};
use uuid::Uuid;

use crate::dto::{
    ArchivePreview, ArchiveRun, MAX_PREVIEW_WARNINGS, PlanPreview, RunReport, human_bytes,
};
use crate::error::{ServiceError, ServiceResult};
use crate::service::{Loaded, RunOptions, Service, check_verdict, elapsed_ms, quota_shortfall};

/// Prefixo dos temporários do app — os únicos arquivos que ele apaga de vez (SEG-10).
pub(crate) const TMP_PREFIX: &str = ".syncly-tmp-";

/// Parâmetros de uma execução compactada, vindos de `run_job`.
pub(crate) struct ArchiveExec<'a> {
    pub format: ArchiveFormat,
    pub retention: Retention,
    pub options: RunOptions,
    pub cancel: &'a Arc<AtomicBool>,
    pub progress: &'a (dyn Fn(&Progress) + Send + Sync),
    pub started: Instant,
}

/// Tudo o que a prévia e a execução precisam, calculado sem alterar nada.
struct Prepared {
    loaded: Loaded,
    provider: Box<dyn StorageProvider>,
    /// Compartilhado com a compactação, que roda numa thread à parte.
    scan: Arc<ScanResult>,
    fingerprint: String,
    /// Versões registradas neste destino, da mais nova para a mais antiga.
    versions: Vec<ArchiveRecord>,
    unchanged: bool,
    verdict: GuardVerdict,
    first_run: bool,
}

impl Prepared {
    /// (arquivos, pastas, bytes antes de compactar)
    fn totals(&self) -> (u64, u64, u64) {
        self.scan
            .entries
            .values()
            .fold((0, 0, 0), |(files, folders, bytes), e| match e.kind {
                EntryKind::File => (files + 1, folders, bytes + e.size),
                EntryKind::Folder => (files, folders + 1, bytes),
            })
    }
}

/// Arquivo temporário apagado ao sair do escopo, com sucesso ou erro.
pub(crate) struct TempFile(pub PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        // Só temporários do próprio app (regra 1 do AGENTS.md).
        if is_app_temp(&self.0) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
}

fn is_app_temp(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with(TMP_PREFIX))
}

impl Service {
    async fn prepare_archive(
        &self,
        loaded: Loaded,
        format: ArchiveFormat,
    ) -> ServiceResult<Prepared> {
        let provider = self.provider_for(&loaded.account).await?;
        let state = self.state()?;
        let versions = state.list_archives(
            loaded.job.id,
            loaded.account.id,
            loaded.sync.destination.path.as_str(),
        )?;
        let first_run = !state.has_successful_run(loaded.job.id, RunKind::Backup)?;
        drop(state);

        // RN-64: mesmos filtros da sincronização. Sem hash — a impressão digital usa
        // tamanho e data, e o conteúdo é lido uma vez só, ao compactar.
        let request = ScanRequest {
            sources: loaded.job.sources.clone(),
            filter: Filter::new(&loaded.job.exclude, loaded.job.include_sensitive)
                .map_err(ServiceError::Invalid)?,
            skip_dirs: self.paths.app_dirs(),
            hash_kind: None,
        };
        let scan =
            tokio::task::spawn_blocking(move || scanner::scan(&request, &Default::default()))
                .await
                .map_err(|e| ServiceError::Internal(e.to_string()))?;
        let fingerprint = fingerprint(&scan);

        // RN-59: igual à última versão, e ela continua lá → nada a enviar.
        let unchanged = match versions.first() {
            Some(latest)
                if latest.fingerprint == fingerprint && latest.format == format.as_str() =>
            {
                is_intact(provider.as_ref(), latest).await?
            }
            _ => false,
        };
        let verdict = guard::evaluate_archive(&ArchiveGuardInput {
            sources: &scan.sources,
            entries: scan.entries.len(),
            has_versions: !versions.is_empty(),
            unchanged,
            first_run,
        });
        Ok(Prepared {
            loaded,
            provider,
            scan: Arc::new(scan),
            fingerprint,
            versions,
            unchanged,
            verdict,
            first_run,
        })
    }

    pub(crate) async fn preview_archive(
        &self,
        loaded: Loaded,
        format: ArchiveFormat,
        retention: Retention,
        started: Instant,
    ) -> ServiceResult<PlanPreview> {
        let p = self.prepare_archive(loaded, format).await?;
        let now = Local::now().naive_local();
        let (files, folders, bytes) = p.totals();
        let to_trash = if p.unchanged {
            Vec::new()
        } else {
            expired(&p.versions, Some(now), &retention)
                .into_iter()
                .map(|v| v.name.clone())
                .collect()
        };
        Ok(PlanPreview {
            job_id: p.loaded.job.id,
            mode: p.loaded.sync.mode,
            stats: PlanStats::default(),
            verdict: p.verdict,
            actions: Vec::new(),
            total_actions: 0,
            warnings: p
                .scan
                .warnings
                .iter()
                .map(|w| w.describe())
                .take(MAX_PREVIEW_WARNINGS)
                .collect(),
            hashed_files: 0,
            duration_ms: elapsed_ms(started),
            first_run: p.first_run,
            archive: Some(ArchivePreview {
                name: version_name(&p.loaded.job.name, now, format, |_| false),
                format,
                files,
                folders,
                bytes,
                unchanged: p.unchanged,
                versions: p.versions.len(),
                to_trash,
            }),
        })
    }

    /// Compacta, envia, registra a versão e aplica a retenção (RN-50 a RN-59).
    pub(crate) async fn run_archive(
        &self,
        loaded: Loaded,
        exec: ArchiveExec<'_>,
    ) -> ServiceResult<RunReport> {
        let id = loaded.job.id;
        let trigger = exec.options.trigger;
        let p = self.prepare_archive(loaded, exec.format).await?;
        let state = self.state()?;
        check_verdict(&state, id, RunKind::Backup, &p.verdict, exec.options)?;

        if p.unchanged {
            let (files, _, bytes_in) = p.totals();
            let run = state.start_run(id, RunKind::Backup, trigger)?;
            let report = ExecutionReport::default();
            let stats = serde_json::to_value(&report).ok();
            state.finish_run(run, RunStatus::Success, stats.as_ref(), None)?;
            tracing::info!(job = %id, "sem mudanças desde a última versão");
            let mut out = RunReport::new(RunStatus::Success, report, elapsed_ms(exec.started));
            out.archive = Some(ArchiveRun {
                name: None,
                files,
                bytes_in,
                bytes_out: 0,
                unchanged: true,
                versions: p.versions.len(),
            });
            return Ok(out);
        }

        // RN-53, RN-67: o temporário nunca passa da soma das entradas (+ pequena sobra).
        let staging = self.staging_dir(id)?;
        let (_, _, estimated) = p.totals();
        if let Ok(available) = fs4::available_space(&staging)
            && available < estimated.saturating_add(DISK_RESERVE_BYTES)
        {
            let msg = format!(
                "o disco do computador tem {} livres; compactar pode precisar de até {} \
                 (reserva de {})",
                human_bytes(available),
                human_bytes(estimated),
                human_bytes(DISK_RESERVE_BYTES)
            );
            let run = state.start_run(id, RunKind::Backup, trigger)?;
            state.finish_run(run, RunStatus::Aborted, None, Some(&msg))?;
            return Err(CoreError::DiskFull(msg).into());
        }

        let run_id = state.start_run(id, RunKind::Backup, trigger)?;
        tracing::info!(job = %id, "execução compactada iniciada");
        // A conexão não pode atravessar `.await` sozinha (não é `Sync`).
        let shared = Mutex::new(state);
        let result = self.send_version(&p, &shared, &staging, &exec).await;
        let state = shared.into_inner().unwrap_or_else(PoisonError::into_inner);
        let (status, report, archive) = match result {
            Ok(done) => done,
            Err((status, e)) => {
                state.finish_run(run_id, status, None, Some(&e.to_string()))?;
                tracing::info!(job = %id, status = status.as_str(), "execução compactada interrompida");
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
        let stats = serde_json::to_value(&report).ok();
        state.finish_run(run_id, status, stats.as_ref(), None)?;
        tracing::info!(job = %id, status = status.as_str(), trashed = report.trashed, "execução concluída");
        let mut out = RunReport::new(status, report, elapsed_ms(exec.started));
        out.archive = Some(archive);
        Ok(out)
    }

    /// Compacta, envia e aplica a retenção. Um erro vem com o status que vai para o
    /// histórico (falha, cancelada ou bloqueada).
    async fn send_version(
        &self,
        p: &Prepared,
        state: &Mutex<StateDb>,
        staging: &Path,
        exec: &ArchiveExec<'_>,
    ) -> Result<(RunStatus, ExecutionReport, ArchiveRun), (RunStatus, ServiceError)> {
        let id = p.loaded.job.id;
        let dest = &p.loaded.sync.destination.path;
        let provider = p.provider.as_ref();

        let temp = TempFile(staging.join(format!(
            "{TMP_PREFIX}{}.{}",
            Uuid::new_v4(),
            exec.format.extension()
        )));
        let created = compress(p, &temp.0, exec.format, exec.cancel, exec.progress)
            .await
            .map_err(|e| match e {
                ServiceError::Core(CoreError::Cancelled) => (RunStatus::Cancelled, e),
                e => (RunStatus::Failed, e),
            })?;
        // RN-46: com o tamanho real, já compactado.
        if let Some(msg) = quota_shortfall(provider, &p.loaded.settings, created.bytes_out)
            .await
            .map_err(failed)?
        {
            return Err((RunStatus::Aborted, ServiceError::Invalid(msg)));
        }
        if exec.cancel.load(Ordering::Relaxed) {
            return Err((RunStatus::Cancelled, CoreError::Cancelled.into()));
        }

        let folder = resolve_path(provider, dest, true)
            .await
            .map_err(failed)?
            .ok_or_else(|| failed(ServiceError::Internal("pasta de destino não criada".into())))?;
        let taken = provider
            .child_names(&folder.id)
            .await
            .map_err(|e| failed(CoreError::from(e)))?;
        let now = Utc::now();
        // RN-50, RN-62: nome com data; nunca sobrescreve.
        let name = version_name(
            &p.loaded.job.name,
            now.with_timezone(&Local).naive_local(),
            exec.format,
            |n| taken.iter().any(|t| t == n),
        );
        (exec.progress)(&Progress {
            done: 0,
            total: 1,
            bytes_done: 0,
            bytes_total: created.bytes_out,
            current: Some(format!("enviando {name}")),
        });
        // O provedor confere o checksum (RN-37) e já repete erros passageiros (RN-96).
        let entry = provider
            .upload_new(&folder.id, &name, &temp.0)
            .await
            .map_err(|e| failed(CoreError::from(e)))?;
        drop(temp);
        lock(state)
            .insert_archive(&ArchiveRecord {
                id: Uuid::new_v4(),
                job_id: id,
                account_id: p.loaded.account.id,
                dest_path: dest.to_string(),
                remote_id: entry.id,
                name: name.clone(),
                format: exec.format.as_str().to_string(),
                size: entry.size.unwrap_or(created.bytes_out),
                hash: entry.hash,
                fingerprint: p.fingerprint.clone(),
                created_at: now,
            })
            .map_err(failed)?;
        (exec.progress)(&Progress {
            done: 1,
            total: 1,
            bytes_done: created.bytes_out,
            bytes_total: created.bytes_out,
            current: None,
        });

        let mut report = ExecutionReport {
            uploaded: 1,
            bytes_sent: created.bytes_out,
            ..ExecutionReport::default()
        };
        // RN-27: o que não pôde ser lido ficou fora — a execução é parcial.
        report
            .failures
            .extend(p.scan.blocked.iter().map(|path| Failure {
                path: path.clone(),
                error: "não pôde ser lido — ficou fora do arquivo compactado".into(),
            }));
        // RN-54, RN-55: só depois da versão nova enviada e verificada.
        let versions = lock(state)
            .list_archives(id, p.loaded.account.id, dest.as_str())
            .map_err(failed)?;
        for old in expired(&versions, None, &exec.retention) {
            match retire(provider, old).await {
                Ok(trashed) => {
                    lock(state).delete_archive(old.id).map_err(failed)?;
                    report.trashed += u32::from(trashed);
                }
                Err(e) => report.failures.push(Failure {
                    path: old.name.clone(),
                    error: format!("não foi para a lixeira: {e}"),
                }),
            }
        }
        let kept = lock(state)
            .list_archives(id, p.loaded.account.id, dest.as_str())
            .map_err(failed)?
            .len();

        // RN-91
        let status = if report.failures.is_empty() {
            RunStatus::Success
        } else {
            RunStatus::PartialSuccess
        };
        let archive = ArchiveRun {
            name: Some(name),
            files: created.files,
            bytes_in: created.bytes_in,
            bytes_out: created.bytes_out,
            unchanged: false,
            versions: kept,
        };
        Ok((status, report, archive))
    }

    /// Pasta dos temporários da tarefa. Sobras de uma execução interrompida são
    /// apagadas — a trava da tarefa (RN-90) garante que nenhuma outra as usa.
    pub(crate) fn staging_dir(&self, job_id: Uuid) -> ServiceResult<PathBuf> {
        let dir = self.paths.data_dir.join("tmp").join(job_id.to_string());
        ensure_private_dir(&dir)?;
        if let Ok(items) = std::fs::read_dir(&dir) {
            for item in items.flatten() {
                let path = item.path();
                if is_app_temp(&path) && path.is_file() {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        Ok(dir)
    }
}

fn lock(state: &Mutex<StateDb>) -> MutexGuard<'_, StateDb> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn failed(e: impl Into<ServiceError>) -> (RunStatus, ServiceError) {
    (RunStatus::Failed, e.into())
}

/// Compacta fora do runtime assíncrono, repassando o progresso.
async fn compress(
    p: &Prepared,
    output: &Path,
    format: ArchiveFormat,
    cancel: &Arc<AtomicBool>,
    progress: &(dyn Fn(&Progress) + Send + Sync),
) -> ServiceResult<CreateReport> {
    let scan = p.scan.clone();
    let output = output.to_path_buf();
    let cancel = cancel.clone();
    with_progress(progress, move |report| {
        archive::create(&CreateRequest {
            scan: &scan,
            output: &output,
            format,
            level: None,
            cancel: &cancel,
            progress: report,
        })
    })
    .await
}

/// Roda uma compactação/extração numa thread à parte (E/S de disco pesada) e repassa o
/// progresso dela como [`Progress`] — o mesmo evento das execuções de tarefa.
pub(crate) async fn with_progress<T: Send + 'static>(
    progress: &(dyn Fn(&Progress) + Send + Sync),
    work: impl FnOnce(&(dyn Fn(&ArchiveProgress) + Send + Sync)) -> CoreResult<T> + Send + 'static,
) -> ServiceResult<T> {
    let (tx, mut rx) = tokio::sync::watch::channel(ArchiveProgress::default());
    let mut task = tokio::task::spawn_blocking(move || {
        work(&move |a: &ArchiveProgress| {
            tx.send_replace(a.clone());
        })
    });
    let result = loop {
        tokio::select! {
            r = &mut task => break r,
            Ok(()) = rx.changed() => {
                let a = rx.borrow_and_update().clone();
                progress(&Progress {
                    done: usize::try_from(a.files_done).unwrap_or(usize::MAX),
                    total: usize::try_from(a.files_total).unwrap_or(usize::MAX),
                    bytes_done: a.bytes_done,
                    bytes_total: a.bytes_total,
                    current: a.current,
                });
            }
        }
    };
    Ok(result.map_err(|e| ServiceError::Internal(e.to_string()))??)
}

/// A versão registrada continua no destino, com o mesmo nome e tamanho?
/// Uma versão renomeada ou substituída fora do app deixa de ser dele.
pub(crate) async fn is_intact(
    provider: &dyn StorageProvider,
    v: &ArchiveRecord,
) -> ServiceResult<bool> {
    match provider.get(&v.remote_id).await {
        Ok(Some(e)) => Ok(e.kind == EntryKind::File
            && e.name == v.name
            && e.size.is_none_or(|s| s == v.size)
            && (e.hash.is_none() || v.hash.is_none() || e.hash == v.hash)),
        Ok(None) | Err(ProviderError::NotFound(_)) => Ok(false),
        Err(e) => Err(CoreError::from(e).into()),
    }
}

/// RN-55: manda uma versão antiga para a lixeira — só se ainda for a que o app criou.
/// `Ok(false)`: já não estava lá (ou foi alterada por fora); o registro é esquecido.
async fn retire(provider: &dyn StorageProvider, v: &ArchiveRecord) -> ServiceResult<bool> {
    if !is_intact(provider, v).await? {
        tracing::info!("versão antiga sumiu ou foi alterada fora do app — deixada como está");
        return Ok(false);
    }
    match provider.trash(&v.remote_id).await {
        Ok(()) => Ok(true),
        Err(ProviderError::NotFound(_)) => Ok(false),
        Err(e) => Err(CoreError::from(e).into()),
    }
}

/// RN-54: versões que ficam de fora da política (horário local). `upcoming` é a versão
/// que a prévia supõe que será criada agora.
fn expired<'a>(
    versions: &'a [ArchiveRecord],
    upcoming: Option<NaiveDateTime>,
    policy: &Retention,
) -> Vec<&'a ArchiveRecord> {
    let mut dated: Vec<(Uuid, NaiveDateTime)> = versions
        .iter()
        .map(|v| (v.id, v.created_at.with_timezone(&Local).naive_local()))
        .collect();
    if let Some(at) = upcoming {
        dated.push((Uuid::nil(), at));
    }
    let plan = plan_retention(&dated, policy);
    versions
        .iter()
        .filter(|v| plan.remove.contains(&v.id))
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::dto::{JobDraft, SyncDraft};
    use syncly_core::domain::{Delivery, RunTrigger, SyncMode};

    pub(crate) struct Fixture {
        pub _tmp: tempfile::TempDir,
        pub service: Service,
        pub source: PathBuf,
        pub backups: PathBuf,
        pub job: Uuid,
    }

    pub(crate) fn fixture(retention: Retention) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let service = crate::test_support::service(&tmp.path().join("app"), "http://127.0.0.1:9");
        let source = tmp.path().join("Mundo");
        std::fs::create_dir_all(source.join("region")).unwrap();
        std::fs::write(source.join("level.dat"), "nivel").unwrap();
        std::fs::write(source.join("region/r.0.mca"), "regiao").unwrap();
        let hd = tmp.path().join("hd");
        std::fs::create_dir_all(&hd).unwrap();
        let account = service.add_local_account("HD".into(), hd.clone()).unwrap();
        let job = service
            .save_job(JobDraft {
                id: None,
                name: "Mundo".into(),
                enabled: true,
                sources: vec![source.clone()],
                exclude: vec![],
                include_sensitive: false,
                sync: Some(SyncDraft {
                    account_id: account.id,
                    path: "Backups".into(),
                    delivery: Delivery::Archive {
                        format: ArchiveFormat::Zip,
                        retention,
                    },
                    mode: SyncMode::Upload,
                }),
            })
            .unwrap()
            .id;
        Fixture {
            _tmp: tmp,
            service,
            source,
            backups: hd.join("Backups"),
            job,
        }
    }

    pub(crate) fn options(confirm: bool) -> RunOptions {
        RunOptions {
            confirm,
            trigger: RunTrigger::Manual,
        }
    }

    pub(crate) fn names(dir: &Path) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        out.sort();
        out
    }

    fn only_last(keep_last: u16) -> Retention {
        Retention {
            keep_last,
            daily: 0,
            weekly: 0,
            monthly: 0,
        }
    }

    #[tokio::test]
    async fn rn50_rn59_envia_versao_e_pula_quando_nada_mudou() {
        let f = fixture(Retention::default());

        let preview = f.service.preview_job(f.job).await.unwrap();
        let archive = preview.archive.unwrap();
        assert_eq!((archive.files, archive.folders), (2, 2));
        assert!(archive.name.starts_with("Mundo_") && archive.name.ends_with(".zip"));
        assert!(!archive.unchanged);
        // RN-36: primeira vez pede confirmação; a prévia não altera nada.
        assert!(matches!(
            preview.verdict,
            GuardVerdict::NeedsConfirmation(_)
        ));
        assert!(!f.backups.exists());
        let err = f
            .service
            .run_job(f.job, options(false), |_| {})
            .await
            .unwrap_err();
        assert_eq!(err.code(), "needs_confirmation");

        let progress = std::sync::Mutex::new(0);
        let report = f
            .service
            .run_job(f.job, options(true), |_| *progress.lock().unwrap() += 1)
            .await
            .unwrap();
        assert_eq!(report.status, "success");
        assert_eq!(report.report.uploaded, 1);
        assert!(*progress.lock().unwrap() > 0);
        let sent = report.archive.unwrap();
        let name = sent.name.unwrap();
        assert_eq!(names(&f.backups), std::slice::from_ref(&name));
        assert_eq!(sent.versions, 1);

        // O conteúdo enviado descompacta igual à fonte.
        let out = f._tmp.path().join("restaurado");
        std::fs::create_dir_all(&out).unwrap();
        let extracted = f
            .service
            .extract(f.backups.join(&name), out, |_| {})
            .await
            .unwrap();
        let root = PathBuf::from(extracted.dest);
        assert_eq!(
            std::fs::read_to_string(root.join("level.dat")).unwrap(),
            "nivel"
        );

        // RN-59: nada mudou → nada é enviado, e não pede confirmação de novo.
        let report = f
            .service
            .run_job(f.job, options(false), |_| {})
            .await
            .unwrap();
        let again = report.archive.unwrap();
        assert!(again.unchanged && again.name.is_none());
        assert_eq!(names(&f.backups).len(), 1);

        // Mudou → nova versão, a anterior fica (retenção padrão).
        std::fs::write(f.source.join("level.dat"), "nivel 2").unwrap();
        let report = f
            .service
            .run_job(f.job, options(false), |_| {})
            .await
            .unwrap();
        assert_eq!(report.archive.unwrap().versions, 2);
        assert_eq!(names(&f.backups).len(), 2);

        // Nenhum temporário sobra no computador.
        let staging = f.service.paths.data_dir.join("tmp").join(f.job.to_string());
        assert!(names(&staging).is_empty());
        let last = f.service.list_jobs().unwrap()[0].last_run.clone().unwrap();
        assert_eq!((last.kind, last.status), ("backup", "success"));
    }

    #[tokio::test]
    async fn rn54_versoes_alem_da_retencao_vao_para_a_lixeira() {
        let f = fixture(only_last(1));
        f.service
            .run_job(f.job, options(true), |_| {})
            .await
            .unwrap();
        let first = names(&f.backups);

        std::fs::write(f.source.join("level.dat"), "nivel 2").unwrap();
        let preview = f.service.preview_job(f.job).await.unwrap();
        assert_eq!(preview.archive.unwrap().to_trash, first);

        let report = f
            .service
            .run_job(f.job, options(false), |_| {})
            .await
            .unwrap();
        assert_eq!(report.report.trashed, 1);
        assert_eq!(report.archive.unwrap().versions, 1);
        let now = names(&f.backups);
        assert_eq!(now.len(), 1);
        assert_ne!(now, first);
        // SEG-10: foi para a lixeira do destino, não sumiu.
        let trash = f.backups.parent().unwrap().join(".syncly-trash");
        let bucket = std::fs::read_dir(&trash)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(names(&bucket), first);
    }

    #[tokio::test]
    async fn rn55_versao_mexida_fora_do_app_nao_vai_para_a_lixeira() {
        let f = fixture(only_last(1));
        f.service
            .run_job(f.job, options(true), |_| {})
            .await
            .unwrap();
        let first = names(&f.backups).remove(0);
        std::fs::rename(f.backups.join(&first), f.backups.join("meu.zip")).unwrap();

        std::fs::write(f.source.join("level.dat"), "nivel 2").unwrap();
        let report = f
            .service
            .run_job(f.job, options(false), |_| {})
            .await
            .unwrap();
        assert_eq!(report.status, "success");
        assert_eq!(report.report.trashed, 0);
        assert!(names(&f.backups).contains(&"meu.zip".to_string()));
    }

    #[tokio::test]
    async fn rn43_agendada_nunca_confirma_sozinha() {
        let f = fixture(Retention::default());
        let scheduled = RunOptions {
            confirm: true,
            trigger: RunTrigger::Scheduled,
        };
        let err = f
            .service
            .run_job(f.job, scheduled, |_| {})
            .await
            .unwrap_err();
        assert_eq!(err.code(), "needs_confirmation");
        assert!(!f.backups.exists());
        let last = f.service.list_jobs().unwrap()[0].last_run.clone().unwrap();
        assert_eq!(last.status, "awaiting_confirmation");
    }
}
