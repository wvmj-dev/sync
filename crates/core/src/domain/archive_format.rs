//! Formatos de arquivo compactado (RN-60). Usado pela compactação avulsa e pelas
//! tarefas que enviam compactado (RN-50).

use serde::{Deserialize, Serialize};

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

    /// Mesmo texto da serialização (`zip`, `tar`, `tar_gz`, `tar_zst`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Tar => "tar",
            Self::TarGz => "tar_gz",
            Self::TarZst => "tar_zst",
        }
    }

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
    fn texto_igual_ao_serde() {
        for format in ArchiveFormat::ALL {
            let json = serde_json::to_string(&format).unwrap();
            assert_eq!(json, format!("\"{}\"", format.as_str()));
        }
    }
}
