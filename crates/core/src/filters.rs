//! Exclusões no formato `.gitignore` (RN-20 a RN-23).

use std::path::Path;

use ignore::gitignore::{Gitignore, GitignoreBuilder};

/// RN-21: lixo de sistema e temporários — sempre excluídos.
pub const DEFAULT_EXCLUDES: &[&str] = &[
    ".DS_Store",
    "Thumbs.db",
    "desktop.ini",
    "*.tmp",
    "*.swp",
    "*~",
    "~$*",
    ".~lock.*#",
    ".Trash-*/",
    ".syncly-*",
];

/// RN-22: segredos comuns — excluídos salvo `include_sensitive = true`.
pub const SENSITIVE_EXCLUDES: &[&str] = &[
    ".ssh/",
    ".gnupg/",
    ".env",
    ".env.*",
    "*.pem",
    "*.key",
    "*.p12",
    "*.pfx",
    "id_rsa*",
    "id_ed25519*",
    "*.kdbx",
    ".netrc",
    ".pgpass",
];

/// RN-23: exclusões adicionais por pasta.
pub const IGNORE_FILE_NAME: &str = ".syncignore";

#[derive(Debug, Clone)]
pub struct Filter {
    matcher: Gitignore,
}

impl Filter {
    pub fn new(extra: &[String], include_sensitive: bool) -> Result<Self, String> {
        let mut builder = GitignoreBuilder::new("");
        let defaults = DEFAULT_EXCLUDES.iter();
        let sensitive = SENSITIVE_EXCLUDES.iter().filter(|_| !include_sensitive);
        for pattern in defaults.chain(sensitive) {
            builder
                .add_line(None, pattern)
                .map_err(|e| format!("padrão interno '{pattern}': {e}"))?;
        }
        for pattern in extra {
            if pattern.chars().any(char::is_control) {
                return Err(format!("padrão inválido: {pattern:?}"));
            }
            builder
                .add_line(None, pattern)
                .map_err(|e| format!("padrão '{pattern}' inválido: {e}"))?;
        }
        let matcher = builder
            .build()
            .map_err(|e| format!("exclusões inválidas: {e}"))?;
        Ok(Self { matcher })
    }

    /// `rel` é relativo à raiz da fonte (RN-20).
    pub fn is_excluded(&self, rel: &Path, is_dir: bool) -> bool {
        if rel.as_os_str().is_empty() {
            return false;
        }
        self.matcher
            .matched_path_or_any_parents(rel, is_dir)
            .is_ignore()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn excluded(f: &Filter, path: &str, dir: bool) -> bool {
        f.is_excluded(Path::new(path), dir)
    }

    #[test]
    fn rn21_padroes_sempre_ativos() {
        let f = Filter::new(&[], true).unwrap();
        assert!(excluded(&f, "a/.DS_Store", false));
        assert!(excluded(&f, "relatorio.docx~", false));
        assert!(excluded(&f, "x/.syncly-tmp-123", false));
        assert!(!excluded(&f, "a/relatorio.docx", false));
    }

    #[test]
    fn rn22_sensiveis_excluidos_por_padrao() {
        let f = Filter::new(&[], false).unwrap();
        assert!(excluded(&f, ".ssh/id_ed25519", false));
        assert!(excluded(&f, "projeto/.env", false));
        assert!(excluded(&f, "certs/servidor.key", false));
        let f = Filter::new(&[], true).unwrap();
        assert!(!excluded(&f, "projeto/.env", false));
    }

    #[test]
    fn rn20_padroes_do_usuario() {
        let f = Filter::new(&["node_modules/".into(), "/build".into()], false).unwrap();
        assert!(excluded(&f, "app/node_modules/x/y.js", false));
        assert!(excluded(&f, "build/out.bin", false));
        // Com '/', vale só a partir da raiz da fonte.
        assert!(!excluded(&f, "app/build/out.bin", false));
    }

    #[test]
    fn rejeita_padrao_com_controle() {
        assert!(Filter::new(&["a\nb".into()], false).is_err());
    }
}
