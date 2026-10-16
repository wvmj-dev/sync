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
    self, ArchiveFormat, ArchiveProgress, CreateRequest, DISK_RESERVE_BYTES, ExtractReport,
    ExtractRequest, SkippedEntry,
};
use syncly_core::crypto::{self, SecretKey};
use syncly_core::domain::Job;
use syncly_core::domain::rel_path::validate_name;
use syncly_core::filters::Filter;
use syncly_core::fsutil::unique_path;
use syncly_core::sync::scanner::{self, ScanRequest, ScanResult};

use crate::dto::human_bytes;
use crate::error::{ServiceError, ServiceResult};
use crate::service::{Service, elapsed_ms};

const MAX_INPUTS: usize = 32;
pub(crate) const MAX_REPORTED: usize = 50;

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
    /// RN-70: cifra com a chave ativa (só TAR.ZST, RN-63) — gera `.tar.zst.age`.
    #[serde(default)]
    pub encrypt: bool,
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
pub(crate) struct Slot<'a>(&'a Mutex<Option<Arc<AtomicBool>>>);

impl<'a> Slot<'a> {
    pub(crate) fn take(
        holder: &'a Mutex<Option<Arc<AtomicBool>>>,
    ) -> ServiceResult<(Self, Arc<AtomicBool>)> {
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

pub(crate) fn absolute_dir(path: &Path, what: &str) -> ServiceResult<()> {
    if path.is_absolute() && path.is_dir() {
        Ok(())
    } else {
        Err(ServiceError::Invalid(format!(
            "{what} '{}' não é uma pasta existente",
            path.display()
        )))
    }
}

pub(crate) async fn blocking<T: Send + 'static>(
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
        let name = check_inputs(&req.inputs, req.name.as_deref())?;
        absolute_dir(&req.output_dir, "a pasta de saída")?;
        // RN-62: o arquivo não pode ser gerado dentro do que está sendo compactado.
        if req.inputs.iter().any(|i| req.output_dir.starts_with(i)) {
            return Err(ServiceError::Invalid(
                "a pasta de saída não pode ficar dentro do que está sendo compactado".into(),
            ));
        }

        // RN-63, RN-73: só a chave pública, e só TAR.ZST.
        let key = match req.encrypt {
            true if req.format != ArchiveFormat::TarZst => {
                return Err(ServiceError::Invalid(
                    "a criptografia exige o formato TAR.ZST".into(),
                ));
            }
            true => Some(self.encryption_public_key()?),
            false => None,
        };

        let (_slot, cancel) = Slot::take(&self.archive_op)?;
        let scan = self
            .scan_inputs(req.inputs.clone(), req.include_sensitive)
            .await?;
        ensure_space(&req.output_dir, total_size(&scan), "a pasta de saída")?;

        let extension = archive_extension(req.format, key.is_some());
        let output = unique_path(&req.output_dir, &name, &extension);
        let warnings: Vec<String> = scan.warnings.iter().map(|w| w.describe()).collect();
        let format = req.format;
        let report = blocking(move || {
            archive::create(&CreateRequest {
                scan: &scan,
                output: &output,
                format,
                level: None,
                encrypt: key.as_ref(),
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

    /// RN-64: mesmos filtros da sincronização (lixo, sensíveis, `.syncignore`, links).
    /// Se nada sobrar, é erro — nunca se cria um compactado vazio.
    pub(crate) async fn scan_inputs(
        &self,
        inputs: Vec<PathBuf>,
        include_sensitive: bool,
    ) -> ServiceResult<ScanResult> {
        let request = ScanRequest {
            sources: inputs,
            filter: Filter::new(&[], include_sensitive).map_err(ServiceError::Invalid)?,
            skip_dirs: self.paths.app_dirs(),
            hash_kind: None,
        };
        let scan = blocking(move || Ok(scanner::scan(&request, &Default::default()))).await?;
        if scan.entries.is_empty() {
            return Err(ServiceError::Invalid(
                "nada para compactar (tudo foi excluído pelos filtros ou não pôde ser lido)".into(),
            ));
        }
        Ok(scan)
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
        let name = ArchiveName::parse(file_name, "extraido")?;
        let keys = self.keys_for(file_name).await?;

        let (_slot, cancel) = Slot::take(&self.archive_op)?;
        let report =
            extract_new_folder(archive_path, &dest_dir, &name.stem, keys, cancel, progress).await?;
        Ok(ExtractResult::from_report(report, started))
    }
}

impl ExtractResult {
    pub(crate) fn from_report(report: ExtractReport, started: Instant) -> Self {
        let skipped_count = report.skipped.len();
        Self {
            dest: report.dest.display().to_string(),
            files: report.files,
            folders: report.folders,
            bytes: report.bytes,
            skipped: report.skipped.into_iter().take(MAX_REPORTED).collect(),
            skipped_count,
            duration_ms: elapsed_ms(started),
        }
    }
}

/// Formato, nome sem extensão e se o arquivo é criptografado (`.age`, RN-63).
pub(crate) struct ArchiveName {
    pub format: ArchiveFormat,
    /// RN-65: a pasta nova leva o nome do arquivo sem a extensão (ou `fallback`).
    pub stem: String,
    pub encrypted: bool,
}

impl ArchiveName {
    pub(crate) fn parse(file_name: &str, fallback: &str) -> ServiceResult<Self> {
        let (encrypted, inner) = match crypto::strip_extension(file_name) {
            Some(inner) => (true, inner),
            None => (false, file_name),
        };
        let Some((format, stem)) = ArchiveFormat::detect(inner) else {
            return Err(ServiceError::Invalid(
                "formato não suportado — use .zip, .tar, .tar.gz, .tar.zst ou .tar.zst.age".into(),
            ));
        };
        Ok(Self {
            format,
            stem: if validate_name(stem).is_ok() {
                stem.to_string()
            } else {
                fallback.to_string()
            },
            encrypted,
        })
    }

    /// Extensão completa, com `.age` se for criptografado.
    pub(crate) fn extension(&self) -> String {
        archive_extension(self.format, self.encrypted)
    }
}

pub(crate) fn archive_extension(format: ArchiveFormat, encrypted: bool) -> String {
    if encrypted {
        format!("{}.{}", format.extension(), crypto::EXTENSION)
    } else {
        format.extension().to_string()
    }
}

/// RN-65 a RN-67: descompacta numa pasta nova dentro de `dest_dir` — nunca por cima de
/// arquivos existentes —, tratando o conteúdo como não confiável.
pub(crate) async fn extract_new_folder(
    archive_path: PathBuf,
    dest_dir: &Path,
    stem: &str,
    keys: Vec<SecretKey>,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(&ArchiveProgress) + Send + Sync + 'static,
) -> ServiceResult<ExtractReport> {
    let dest = unique_path(dest_dir, stem, "");
    let report = blocking(move || {
        archive::extract(&ExtractRequest {
            archive: &archive_path,
            dest: &dest,
            reserve_bytes: DISK_RESERVE_BYTES,
            keys: &keys,
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
    Ok(report)
}

/// RN-60: de 1 a 32 entradas existentes, com nomes únicos (viram o primeiro nível do
/// arquivo). Retorna o nome do arquivo sem extensão: o pedido, o da única entrada, ou
/// "arquivos".
pub(crate) fn check_inputs(inputs: &[PathBuf], name: Option<&str>) -> ServiceResult<String> {
    if inputs.is_empty() || inputs.len() > MAX_INPUTS {
        return Err(ServiceError::Invalid(format!(
            "escolha de 1 a {MAX_INPUTS} pastas ou arquivos"
        )));
    }
    let mut names = HashSet::new();
    for input in inputs {
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
    let name = match name {
        Some(n) if !n.trim().is_empty() => n.trim().to_string(),
        _ if inputs.len() == 1 => Job::source_name(&inputs[0])
            .unwrap_or("arquivos")
            .to_string(),
        _ => "arquivos".to_string(),
    };
    validate_name(&name).map_err(ServiceError::Invalid)?;
    Ok(name)
}

/// Soma dos tamanhos: o compactado nunca passa disso (+ cabeçalhos).
pub(crate) fn total_size(scan: &ScanResult) -> u64 {
    scan.entries.values().map(|e| e.size).sum()
}

/// RN-67: `needed` bytes cabem em `dir` sem passar da reserva de 1 GiB.
pub(crate) fn ensure_space(dir: &Path, needed: u64, what: &str) -> ServiceResult<()> {
    match fs4::available_space(dir) {
        Ok(available) if available < needed.saturating_add(DISK_RESERVE_BYTES) => {
            Err(CoreError::DiskFull(format!(
                "{what} tem {} livres; a operação pode precisar de até {} (reserva de {})",
                human_bytes(available),
                human_bytes(needed),
                human_bytes(DISK_RESERVE_BYTES)
            ))
            .into())
        }
        _ => Ok(()),
    }
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
            encrypt: false,
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
