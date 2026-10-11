//! Compactar e descompactar (RN-60 a RN-67). As operações rodam fora do runtime assíncrono
//! (são E/S de disco pesada) e uma por vez, com progresso e cancelamento.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use syncly_core::CoreError;
use syncly_core::archive::{
    self, ArchiveFormat, ArchiveProgress, CreateRequest, DISK_RESERVE_BYTES, ExtractRequest,
    SkippedEntry,
};
use syncly_core::domain::Job;
use syncly_core::domain::rel_path::validate_name;
use syncly_core::filters::Filter;
use syncly_core::fsutil::unique_path;
use syncly_core::sync::scanner::{self, ScanRequest};

use crate::dto::human_bytes;
use crate::error::{ServiceError, ServiceResult};
use crate::service::Service;

const MAX_INPUTS: usize = 32;
const MAX_REPORTED: usize = 50;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressRequest {
    pub inputs: Vec<PathBuf>,
    pub output_dir: PathBuf,
    /// Nome sem extensão; padrão: nome da única entrada, ou "arquivos".
    #[serde(default)]
    pub name: Option<String>,
    pub format: ArchiveFormat,
    /// RN-64: arquivos sensíveis só entram se marcado.
    #[serde(default)]
    pub include_sensitive: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressResult {
    pub path: String,
    pub format: ArchiveFormat,
    pub files: u64,
    pub folders: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub warnings: Vec<String>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractResult {
    pub dest: String,
    pub files: u64,
    pub folders: u64,
    pub bytes: u64,
    /// RN-66: entradas ignoradas por segurança (até 50; total em `skipped_count`).
    pub skipped: Vec<SkippedEntry>,
    pub skipped_count: usize,
    pub duration_ms: u64,
}

/// Uma operação de arquivo por vez; libera o lugar ao terminar, mesmo com erro.
struct Slot<'a>(&'a Mutex<Option<Arc<AtomicBool>>>);

impl<'a> Slot<'a> {
    fn take(holder: &'a Mutex<Option<Arc<AtomicBool>>>) -> ServiceResult<(Self, Arc<AtomicBool>)> {
        let mut current = holder.lock().unwrap_or_else(PoisonError::into_inner);
        if current.is_some() {
            return Err(ServiceError::Busy(
                "já existe uma compactação/descompactação em andamento",
            ));
        }
        let flag = Arc::new(AtomicBool::new(false));
        *current = Some(flag.clone());
        Ok((Self(holder), flag))
    }
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

fn absolute_dir(path: &Path, what: &str) -> ServiceResult<()> {
    if path.is_absolute() && path.is_dir() {
        Ok(())
    } else {
        Err(ServiceError::Invalid(format!(
            "{what} '{}' não é uma pasta existente",
            path.display()
        )))
    }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, CoreError> + Send + 'static,
) -> ServiceResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ServiceError::Internal(e.to_string()))?
        .map_err(ServiceError::from)
}

impl Service {
    pub fn cancel_archive(&self) -> bool {
        let current = self
            .archive_op
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match current.as_ref() {
            Some(flag) => {
                flag.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// Compacta pastas/arquivos num `.zip` ou `.tar[.gz|.zst]` (RN-60 a RN-64, RN-67).
    pub async fn compress(
        &self,
        req: CompressRequest,
        progress: impl Fn(&ArchiveProgress) + Send + Sync + 'static,
    ) -> ServiceResult<CompressResult> {
        let started = Instant::now();
        if req.inputs.is_empty() || req.inputs.len() > MAX_INPUTS {
            return Err(ServiceError::Invalid(format!(
                "escolha de 1 a {MAX_INPUTS} pastas ou arquivos"
            )));
        }
        let mut names = HashSet::new();
        for input in &req.inputs {
            if !input.is_absolute() || !input.exists() {
                return Err(ServiceError::Invalid(format!(
                    "'{}' não existe",
                    input.display()
                )));
            }
            let name = Job::source_name(input)
                .filter(|n| validate_name(n).is_ok())
                .ok_or_else(|| {
                    ServiceError::Invalid(format!("'{}' não tem um nome válido", input.display()))
                })?;
            if !names.insert(name.to_lowercase()) {
                return Err(ServiceError::Invalid(format!(
                    "duas entradas com o mesmo nome '{name}'"
                )));
            }
        }
        absolute_dir(&req.output_dir, "a pasta de saída")?;
        // RN-62: o arquivo não pode ser gerado dentro do que está sendo compactado.
        if req.inputs.iter().any(|i| req.output_dir.starts_with(i)) {
            return Err(ServiceError::Invalid(
                "a pasta de saída não pode ficar dentro do que está sendo compactado".into(),
            ));
        }
        let name = match &req.name {
            Some(n) if !n.trim().is_empty() => n.trim().to_string(),
            _ if req.inputs.len() == 1 => Job::source_name(&req.inputs[0])
                .unwrap_or("arquivos")
                .to_string(),
            _ => "arquivos".to_string(),
        };
        validate_name(&name).map_err(ServiceError::Invalid)?;

        let (_slot, cancel) = Slot::take(&self.archive_op)?;
        // RN-64: mesmos filtros da sincronização (lixo, sensíveis, `.syncignore`, links).
        let scan_request = ScanRequest {
            sources: req.inputs.clone(),
            filter: Filter::new(&[], req.include_sensitive).map_err(ServiceError::Invalid)?,
            skip_dirs: self.paths.app_dirs(),
            hash_kind: None,
        };
        let scan = blocking(move || Ok(scanner::scan(&scan_request, &Default::default()))).await?;
        if scan.entries.is_empty() {
            return Err(ServiceError::Invalid(
                "nada para compactar (tudo foi excluído pelos filtros ou não pôde ser lido)".into(),
            ));
        }

        // RN-67: o resultado nunca é maior que a soma das entradas (+ pequena sobra).
        let estimated: u64 = scan.entries.values().map(|e| e.size).sum();
        if let Ok(available) = fs4::available_space(&req.output_dir)
            && available < estimated.saturating_add(DISK_RESERVE_BYTES)
        {
            return Err(CoreError::DiskFull(format!(
                "a pasta de saída tem {} livres; a compactação pode precisar de até {} (reserva de {})",
                human_bytes(available),
                human_bytes(estimated),
                human_bytes(DISK_RESERVE_BYTES)
            ))
            .into());
        }

        let output = unique_path(&req.output_dir, &name, req.format.extension());
        let warnings: Vec<String> = scan.warnings.iter().map(|w| w.describe()).collect();
        let format = req.format;
        let report = blocking(move || {
            archive::create(&CreateRequest {
                scan: &scan,
                output: &output,
                format,
                level: None,
                cancel: &cancel,
                progress: &progress,
            })
        })
        .await?;
        tracing::info!(files = report.files, "compactação concluída");
        Ok(CompressResult {
            path: report.path.display().to_string(),
            format,
            files: report.files,
            folders: report.folders,
            bytes_in: report.bytes_in,
            bytes_out: report.bytes_out,
            warnings: warnings.into_iter().take(MAX_REPORTED).collect(),
            duration_ms: elapsed_ms(started),
        })
    }

    /// Descompacta numa pasta nova dentro de `dest_dir` (RN-65 a RN-67).
    pub async fn extract(
        &self,
        archive_path: PathBuf,
        dest_dir: PathBuf,
        progress: impl Fn(&ArchiveProgress) + Send + Sync + 'static,
    ) -> ServiceResult<ExtractResult> {
        let started = Instant::now();
        if !archive_path.is_absolute() || !archive_path.is_file() {
            return Err(ServiceError::Invalid(format!(
                "'{}' não é um arquivo existente",
                archive_path.display()
            )));
        }
        absolute_dir(&dest_dir, "a pasta de destino")?;
        let file_name = archive_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let Some((_, stem)) = ArchiveFormat::detect(file_name) else {
            return Err(ServiceError::Invalid(
                "formato não suportado — use .zip, .tar, .tar.gz ou .tar.zst".into(),
            ));
        };
        let stem = if validate_name(stem).is_ok() {
            stem
        } else {
            "extraido"
        };
        // RN-65: sempre numa pasta nova — nunca por cima de arquivos existentes.
        let dest = unique_path(&dest_dir, stem, "");

        let (_slot, cancel) = Slot::take(&self.archive_op)?;
        let report = blocking(move || {
            archive::extract(&ExtractRequest {
                archive: &archive_path,
                dest: &dest,
                reserve_bytes: DISK_RESERVE_BYTES,
                cancel: &cancel,
                progress: &progress,
            })
        })
        .await?;
        tracing::info!(
            files = report.files,
            skipped = report.skipped.len(),
            "descompactação concluída"
        );
        let skipped_count = report.skipped.len();
        Ok(ExtractResult {
            dest: report.dest.display().to_string(),
            files: report.files,
            folders: report.folders,
            bytes: report.bytes,
            skipped: report.skipped.into_iter().take(MAX_REPORTED).collect(),
            skipped_count,
            duration_ms: elapsed_ms(started),
        })
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::service;

    fn request(inputs: Vec<PathBuf>, out: PathBuf, format: ArchiveFormat) -> CompressRequest {
        CompressRequest {
            inputs,
            output_dir: out,
            name: None,
            format,
            include_sensitive: false,
        }
    }

    #[tokio::test]
    async fn compactar_e_descompactar_pelo_servico() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(&tmp.path().join("app"), "http://127.0.0.1:9");
        let src = tmp.path().join("Mundo");
        std::fs::create_dir_all(src.join("region")).unwrap();
        std::fs::write(src.join("region/r.0.mca"), "dados").unwrap();
        let out = tmp.path().join("saida");
        std::fs::create_dir_all(&out).unwrap();

        let first = svc
            .compress(
                request(vec![src.clone()], out.clone(), ArchiveFormat::TarGz),
                |_| {},
            )
            .await
            .unwrap();
        assert!(first.path.ends_with("Mundo.tar.gz"));
        // RN-62: segunda vez ganha outro nome, nada é sobrescrito.
        let second = svc
            .compress(
                request(vec![src], out.clone(), ArchiveFormat::TarGz),
                |_| {},
            )
            .await
            .unwrap();
        assert!(second.path.ends_with("Mundo (2).tar.gz"), "{}", second.path);

        let extracted = svc
            .extract(PathBuf::from(&first.path), out.clone(), |_| {})
            .await
            .unwrap();
        assert!(extracted.dest.ends_with("Mundo"));
        assert_eq!(
            std::fs::read_to_string(out.join("Mundo/region/r.0.mca")).unwrap(),
            "dados"
        );
        // RN-65: de novo, vai para uma pasta nova.
        let again = svc
            .extract(PathBuf::from(&first.path), out, |_| {})
            .await
            .unwrap();
        assert!(again.dest.ends_with("Mundo (2)"), "{}", again.dest);
    }

    #[tokio::test]
    async fn rn62_saida_dentro_da_entrada_e_recusada() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(&tmp.path().join("app"), "http://127.0.0.1:9");
        let src = tmp.path().join("Docs");
        std::fs::create_dir_all(&src).unwrap();
        let err = svc
            .compress(request(vec![src.clone()], src, ArchiveFormat::Zip), |_| {})
            .await
            .unwrap_err();
        assert_eq!(err.code(), "invalid");
    }

    #[tokio::test]
    async fn rn64_so_arquivos_sensiveis_nao_gera_compactado_vazio() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(&tmp.path().join("app"), "http://127.0.0.1:9");
        let env = tmp.path().join(".env");
        std::fs::write(&env, "SEGREDO=1").unwrap();
        let err = svc
            .compress(
                request(vec![env], tmp.path().to_path_buf(), ArchiveFormat::Zip),
                |_| {},
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("nada para compactar"), "{err}");
        assert!(!svc.cancel_archive());
    }
}
