//! Compactar e descompactar (RN-60 a RN-67): ZIP e TAR (`.tar`, `.tar.gz`, `.tar.zst`).
//!
//! - Escrita sempre num temporário, que só vira o arquivo/pasta final no fim — nunca
//!   sobra resultado pela metade e nada existente é sobrescrito (RN-62, RN-65).
//! - Na extração, todo caminho interno é validado: nada sai da pasta de destino, links e
//!   arquivos especiais não são criados (RN-66), e o espaço livre é vigiado (RN-67).

mod create;
mod extract;

use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

pub use create::{CreateReport, CreateRequest, create};
pub use extract::{ExtractReport, ExtractRequest, SkippedEntry, extract};

use crate::domain::rel_path::validate_name;

/// RN-67: espaço que precisa sobrar no disco ao compactar ou descompactar.
pub const DISK_RESERVE_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveFormat {
    /// Compatível com tudo (Windows, macOS, Linux).
    Zip,
    /// Sem compressão.
    Tar,
    /// Padrão no Linux.
    TarGz,
    /// Mais rápido e menor.
    TarZst,
}

impl ArchiveFormat {
    pub const ALL: [Self; 4] = [Self::Zip, Self::Tar, Self::TarGz, Self::TarZst];

    /// Extensão sem o ponto inicial.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Tar => "tar",
            Self::TarGz => "tar.gz",
            Self::TarZst => "tar.zst",
        }
    }

    /// Identifica pelo nome do arquivo. Retorna o formato e o nome sem a extensão.
    pub fn detect(file_name: &str) -> Option<(Self, &str)> {
        let lower = file_name.to_ascii_lowercase();
        let known = [
            (".tar.gz", Self::TarGz),
            (".tgz", Self::TarGz),
            (".tar.zst", Self::TarZst),
            (".tzst", Self::TarZst),
            (".tar", Self::Tar),
            (".zip", Self::Zip),
        ];
        known.into_iter().find_map(|(ext, format)| {
            lower
                .ends_with(ext)
                .then(|| (format, &file_name[..file_name.len() - ext.len()]))
        })
    }
}

/// Progresso de uma compactação/descompactação.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveProgress {
    pub files_done: u64,
    pub files_total: u64,
    pub bytes_done: u64,
    /// Na extração de `.tar.*`, o total só é conhecido no fim (0 = desconhecido).
    pub bytes_total: u64,
    pub current: Option<String>,
}

/// RN-66: converte um caminho interno do arquivo compactado em componentes seguros.
/// `None` se for absoluto, tiver `..`, nomes inválidos ou estiver vazio.
pub fn safe_entry_path(name: &str) -> Option<Vec<String>> {
    // Zips criados no Windows às vezes usam `\` — tratado como separador também.
    let normalized = name.replace('\\', "/");
    if normalized.starts_with('/') || normalized.as_bytes().get(1) == Some(&b':') {
        return None;
    }
    let mut parts = Vec::new();
    for part in normalized.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            p if validate_name(p).is_ok() => parts.push(p.to_string()),
            _ => return None,
        }
    }
    (!parts.is_empty()).then_some(parts)
}

/// Intervalo mínimo (em bytes) entre avisos de progresso dentro de um arquivo grande.
const PROGRESS_STEP: u64 = 4 * 1024 * 1024;

/// Leitura em streaming que conta bytes, avisa o progresso e obedece ao cancelamento
/// (verificado a cada bloco — memória constante, qualquer tamanho de arquivo).
struct Tracked<'a, R> {
    inner: R,
    cancel: &'a AtomicBool,
    progress: &'a mut ArchiveProgress,
    notify: &'a (dyn Fn(&ArchiveProgress) + Send + Sync),
    last: u64,
}

impl<'a, R> Tracked<'a, R> {
    fn new(
        inner: R,
        cancel: &'a AtomicBool,
        progress: &'a mut ArchiveProgress,
        notify: &'a (dyn Fn(&ArchiveProgress) + Send + Sync),
    ) -> Self {
        let last = progress.bytes_done;
        Self {
            inner,
            cancel,
            progress,
            notify,
            last,
        }
    }
}

impl<R: Read> Read for Tracked<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, CANCELLED));
        }
        let n = self.inner.read(buf)?;
        self.progress.bytes_done += n as u64;
        if self.progress.bytes_done - self.last >= PROGRESS_STEP {
            self.last = self.progress.bytes_done;
            (self.notify)(self.progress);
        }
        Ok(n)
    }
}

const CANCELLED: &str = "operação cancelada";

/// Arquivo/pasta temporário ao lado do destino final, no mesmo disco (rename atômico).
fn temp_sibling(final_path: &std::path::Path) -> std::path::PathBuf {
    let dir = final_path.parent().unwrap_or(std::path::Path::new("."));
    dir.join(format!(".syncly-tmp-{}", uuid::Uuid::new_v4()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detecta_formatos() {
        assert_eq!(
            ArchiveFormat::detect("Mundo.tar.gz"),
            Some((ArchiveFormat::TarGz, "Mundo"))
        );
        assert_eq!(
            ArchiveFormat::detect("x.TGZ"),
            Some((ArchiveFormat::TarGz, "x"))
        );
        assert_eq!(
            ArchiveFormat::detect("x.tar.zst"),
            Some((ArchiveFormat::TarZst, "x"))
        );
        assert_eq!(
            ArchiveFormat::detect("x.tar"),
            Some((ArchiveFormat::Tar, "x"))
        );
        assert_eq!(
            ArchiveFormat::detect("fotos.zip"),
            Some((ArchiveFormat::Zip, "fotos"))
        );
        assert_eq!(ArchiveFormat::detect("x.rar"), None);
    }

    #[test]
    fn rn66_caminhos_internos_perigosos_sao_rejeitados() {
        assert_eq!(
            safe_entry_path("./Mundo/region/r.0.mca"),
            Some(vec!["Mundo".into(), "region".into(), "r.0.mca".into()])
        );
        assert_eq!(
            safe_entry_path("a\\b.txt"),
            Some(vec!["a".into(), "b.txt".into()])
        );
        for bad in [
            "../evil.txt",
            "a/../../evil",
            "/etc/passwd",
            "\\windows\\x",
            "C:/x",
            "a/\u{0}/b",
            "",
            "./",
        ] {
            assert_eq!(safe_entry_path(bad), None, "{bad:?}");
        }
    }
}
