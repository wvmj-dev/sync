//! Restaurar uma versão de tarefa compactada (RN-57, RN-58): baixar, conferir o checksum
//! registrado no envio e descompactar numa pasta nova.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use syncly_core::CoreError;
use syncly_core::archive::{self, DISK_RESERVE_BYTES, ExtractReport, ExtractRequest};
use syncly_core::crypto::SecretKey;
use syncly_core::domain::{RunKind, RunStatus, RunTrigger};
use syncly_core::fsutil::unique_path;
use syncly_core::hash::hash_file;
use syncly_core::lock::JobLock;
use syncly_core::provider::StorageProvider;
use syncly_core::sync::executor::Progress;
use uuid::Uuid;

use crate::archive::{ArchiveName, ExtractResult, ensure_space};
use crate::delivery::{TMP_PREFIX, TempFile, failed, is_intact, with_progress};
use crate::dto::VersionItem;
use crate::error::{ServiceError, ServiceResult};
use crate::service::{RunningGuard, Service};

impl Service {
    /// Versões enviadas por uma tarefa, em qualquer destino, da mais nova para a mais antiga.
    pub fn list_versions(&self, job_id: Uuid) -> ServiceResult<Vec<VersionItem>> {
        let config = self.load()?;
        config.job(job_id).ok_or(ServiceError::NotFound("tarefa"))?;
        let versions = self.state()?.list_job_archives(job_id)?;
        Ok(versions
            .into_iter()
            .map(|v| VersionItem {
                account_label: config.account(v.account_id).map(|a| a.label.clone()),
                id: v.id,
                name: v.name,
                size: v.size,
                created_at: v.created_at,
                account_id: v.account_id,
                path: v.dest_path,
                key_fingerprint: v.key_fingerprint,
            })
            .collect())
    }

    /// Baixa uma versão, confere o checksum e a descompacta numa pasta nova dentro de
    /// `dest_dir` (RN-57, RN-58). Conta como execução da tarefa: usa a trava dela — a
    /// retenção não manda a versão para a lixeira no meio do caminho — e vai para o histórico.
    pub async fn restore_version(
        &self,
        job_id: Uuid,
        version_id: Uuid,
        dest_dir: &Path,
        progress: impl Fn(&Progress) + Send + Sync,
    ) -> ServiceResult<ExtractResult> {
        let started = Instant::now();
        if !dest_dir.is_absolute() || !dest_dir.is_dir() {
            return Err(ServiceError::Invalid(format!(
                "'{}' não é uma pasta existente",
                dest_dir.display()
            )));
        }
        let config = self.load()?;
        let job = config
            .job(job_id)
            .cloned()
            .ok_or(ServiceError::NotFound("tarefa"))?;
        // RN-57: dentro de uma fonte, a próxima versão levaria a restauração junto.
        if job.sources.iter().any(|s| dest_dir.starts_with(s)) {
            return Err(ServiceError::Invalid(
                "a pasta de restauração não pode ficar dentro de uma fonte da tarefa".into(),
            ));
        }
        if self
            .paths
            .app_dirs()
            .iter()
            .any(|d| dest_dir.starts_with(d))
        {
            return Err(ServiceError::Invalid(
                "a pasta de restauração não pode ficar dentro das pastas do Syncly".into(),
            ));
        }

        // RN-90
        let _lock = JobLock::try_acquire(&self.paths.lock_dir, job_id)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let _running = RunningGuard::register(&self.running, job_id, cancel.clone());

        let state = self.state()?;
        let version = state
            .list_job_archives(job_id)?
            .into_iter()
            .find(|v| v.id == version_id)
            .ok_or(ServiceError::NotFound("versão"))?;
        let account = config
            .account(version.account_id)
            .cloned()
            .ok_or(ServiceError::NotFound("conta da versão"))?;
        let name = ArchiveName::parse(&version.name, "restauracao")?;
        // RN-73: a chave privada só é lida aqui, se a versão for criptografada.
        let keys = self.keys_for(&version.name).await?;

        let provider = self.provider_for(&account).await?;
        // RN-55: só a versão que o app enviou, do jeito que ele enviou.
        if !is_intact(provider.as_ref(), &version).await? {
            return Err(ServiceError::Invalid(format!(
                "'{}' não está mais no destino ou foi alterada fora do app",
                version.name
            )));
        }
        let staging = self.staging_dir(&job_id.to_string())?;
        // RN-67: o download cabe no disco sem passar da reserva.
        ensure_space(&staging, version.size, "o disco do computador")?;

        let run_id = state.start_run(job_id, RunKind::Restore, RunTrigger::Manual)?;
        tracing::info!(job = %job_id, "restauração iniciada");
        let target = Target {
            dir: dest_dir,
            stem: &name.stem,
            extension: name.extension(),
        };
        let file = RemoteFile {
            id: &version.remote_id,
            name: &version.name,
            size: version.size,
            // RN-58: o checksum registrado no envio.
            expected_hash: version.hash.as_deref(),
        };
        let result = fetch_and_extract(
            provider.as_ref(),
            &file,
            &staging,
            &target,
            keys,
            &cancel,
            &progress,
        )
        .await;
        let report = match result {
            Ok(report) => report,
            Err((status, e)) => {
                state.finish_run(run_id, status, None, Some(&e.to_string()))?;
                tracing::info!(job = %job_id, status = status.as_str(), "restauração interrompida");
                return Err(e);
            }
        };
        // RN-66: entradas ignoradas numa versão do próprio app não deveriam existir.
        let status = if report.skipped.is_empty() {
            RunStatus::Success
        } else {
            RunStatus::PartialSuccess
        };
        let stats = serde_json::to_value(&report).ok();
        state.finish_run(run_id, status, stats.as_ref(), None)?;
        tracing::info!(job = %job_id, files = report.files, status = status.as_str(), "restauração concluída");
        Ok(ExtractResult::from_report(report, started))
    }
}

/// Onde a versão será descompactada: uma pasta nova `stem` dentro de `dir`.
pub(crate) struct Target<'a> {
    pub dir: &'a Path,
    pub stem: &'a str,
    /// Extensão completa do arquivo (`tar.zst.age` se criptografado).
    pub extension: String,
}

/// Arquivo compactado no destino.
pub(crate) struct RemoteFile<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub size: u64,
    /// Se houver, o arquivo baixado precisa ter este checksum.
    pub expected_hash: Option<&'a str>,
}

/// Baixa para `staging`, confere e descompacta numa pasta nova (RN-57, RN-58, RN-65 a
/// RN-67). Usado pela restauração e pelo recebimento. Um erro vem com o status que vai
/// para o histórico (falha ou cancelada).
pub(crate) async fn fetch_and_extract(
    provider: &dyn StorageProvider,
    file: &RemoteFile<'_>,
    staging: &Path,
    target: &Target<'_>,
    keys: Vec<SecretKey>,
    cancel: &Arc<AtomicBool>,
    progress: &(dyn Fn(&Progress) + Send + Sync),
) -> Result<ExtractReport, (RunStatus, ServiceError)> {
    // O temporário mantém a extensão completa: `.age` diz que é preciso decifrar.
    let temp = TempFile(staging.join(format!(
        "{TMP_PREFIX}{}.{}",
        Uuid::new_v4(),
        target.extension
    )));
    progress(&Progress {
        done: 0,
        total: 1,
        bytes_done: 0,
        bytes_total: file.size,
        current: Some(format!("baixando {}", file.name)),
    });
    provider
        .download(file.id, &temp.0)
        .await
        .map_err(|e| failed(CoreError::from(e)))?;
    if cancel.load(Ordering::Relaxed) {
        return Err((RunStatus::Cancelled, CoreError::Cancelled.into()));
    }

    // RN-58: confere o checksum — pega também uma versão substituída no destino por
    // outra de mesmo nome e tamanho.
    if let Some(expected) = file.expected_hash {
        progress(&Progress {
            done: 0,
            total: 1,
            bytes_done: file.size,
            bytes_total: file.size,
            current: Some(format!("conferindo {}", file.name)),
        });
        let path = temp.0.clone();
        let kind = provider.hash_kind();
        let actual = tokio::task::spawn_blocking(move || {
            hash_file(&path, kind).map_err(|source| CoreError::Io { path, source })
        })
        .await
        .map_err(|e| failed(ServiceError::Internal(e.to_string())))?
        .map_err(failed)?;
        if actual != expected {
            return Err(failed(ServiceError::Invalid(format!(
                "'{}' não confere com o que foi enviado (checksum diferente) — nada foi \
                 descompactado",
                file.name
            ))));
        }
    }

    // RN-65, RN-66: sempre numa pasta nova, tratando o conteúdo como não confiável.
    let dest = unique_path(target.dir, target.stem, "");
    let archive_path = temp.0.clone();
    let cancel = cancel.clone();
    with_progress(progress, move |report| {
        archive::extract(&ExtractRequest {
            archive: &archive_path,
            dest: &dest,
            reserve_bytes: DISK_RESERVE_BYTES,
            keys: &keys,
            cancel: &cancel,
            progress: report,
        })
    })
    .await
    .map_err(|e| match e {
        ServiceError::Core(CoreError::Cancelled) => (RunStatus::Cancelled, e),
        e => failed(e),
    })
}

#[cfg(test)]
mod tests {
    use crate::delivery::tests::{fixture, names, options};
    use std::path::PathBuf;
    use syncly_core::domain::Retention;

    #[tokio::test]
    async fn rn57_rn58_restaura_versao_em_pasta_nova() {
        let f = fixture(Retention::default());
        f.service
            .run_job(f.job, options(true), |_| {})
            .await
            .unwrap();
        let versions = f.service.list_versions(f.job).unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].account_label.as_deref(), Some("HD"));
        let stem = versions[0].name.trim_end_matches(".zip").to_string();

        let out = f._tmp.path().join("restaurado");
        std::fs::create_dir_all(&out).unwrap();
        let first = f
            .service
            .restore_version(f.job, versions[0].id, &out, |_| {})
            .await
            .unwrap();
        // Uma pasta só no topo (a fonte) vira a própria pasta da restauração.
        let dest = PathBuf::from(&first.dest);
        assert_eq!(dest, out.join(&stem));
        assert_eq!(
            std::fs::read_to_string(dest.join("level.dat")).unwrap(),
            "nivel"
        );
        assert_eq!(
            std::fs::read_to_string(dest.join("region/r.0.mca")).unwrap(),
            "regiao"
        );
        assert_eq!(first.skipped_count, 0);

        // Nunca por cima: a segunda restauração vai para outra pasta.
        let second = f
            .service
            .restore_version(f.job, versions[0].id, &out, |_| {})
            .await
            .unwrap();
        assert_eq!(PathBuf::from(second.dest), out.join(format!("{stem} (2)")));

        let staging = f.service.paths.data_dir.join("tmp").join(f.job.to_string());
        assert!(names(&staging).is_empty());
        let last = f.service.list_jobs().unwrap()[0].last_run.clone().unwrap();
        assert_eq!((last.kind, last.status), ("restore", "success"));
    }

    #[tokio::test]
    async fn rn58_versao_adulterada_nao_e_descompactada() {
        let f = fixture(Retention::default());
        f.service
            .run_job(f.job, options(true), |_| {})
            .await
            .unwrap();
        let version = f.service.list_versions(f.job).unwrap().remove(0);
        // Mesmo nome e tamanho, conteúdo diferente.
        let path = f.backups.join(&version.name);
        let size = std::fs::metadata(&path).unwrap().len();
        std::fs::write(&path, vec![b'x'; usize::try_from(size).unwrap()]).unwrap();

        let out = f._tmp.path().join("restaurado");
        std::fs::create_dir_all(&out).unwrap();
        let err = f
            .service
            .restore_version(f.job, version.id, &out, |_| {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("checksum"), "{err}");
        assert!(names(&out).is_empty());
        let last = f.service.list_jobs().unwrap()[0].last_run.clone().unwrap();
        assert_eq!(last.status, "failed");
    }

    #[tokio::test]
    async fn rn57_nao_restaura_dentro_da_fonte_nem_sem_a_versao() {
        let f = fixture(Retention::default());
        f.service
            .run_job(f.job, options(true), |_| {})
            .await
            .unwrap();
        let version = f.service.list_versions(f.job).unwrap().remove(0);
        let err = f
            .service
            .restore_version(f.job, version.id, &f.source, |_| {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("fonte"), "{err}");

        // Versão removida fora do app: nada a restaurar.
        std::fs::remove_file(f.backups.join(&version.name)).unwrap();
        let out = f._tmp.path().join("restaurado");
        std::fs::create_dir_all(&out).unwrap();
        let err = f
            .service
            .restore_version(f.job, version.id, &out, |_| {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("não está mais"), "{err}");
    }
}
