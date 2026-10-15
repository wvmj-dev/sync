//! Onde o app guarda seus dados (padrão XDG no Linux). Ver ARQUITETURA.md §5.

use std::path::{Path, PathBuf};

use crate::error::{CoreError, CoreResult};
use crate::fsutil::ensure_private_dir;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
    pub lock_dir: PathBuf,
}

impl AppPaths {
    /// Diretórios padrão do usuário: `~/.config/syncly`, `~/.local/share/syncly`…
    pub fn discover() -> CoreResult<Self> {
        let dirs = directories::ProjectDirs::from("dev", "syncly", "Syncly")
            .ok_or(CoreError::NoHomeDir)?;
        let data_dir = dirs.data_dir().to_path_buf();
        let log_dir = dirs
            .state_dir()
            .map_or_else(|| data_dir.join("logs"), |d| d.join("logs"));
        let lock_dir = dirs
            .runtime_dir()
            .map_or_else(|| data_dir.join("locks"), Path::to_path_buf);
        Ok(Self {
            config_dir: dirs.config_dir().to_path_buf(),
            data_dir,
            log_dir,
            lock_dir,
        })
    }

    /// Tudo dentro de um único diretório — para testes ou modo portátil.
    pub fn in_dir(base: &Path) -> Self {
        Self {
            config_dir: base.join("config"),
            data_dir: base.join("data"),
            log_dir: base.join("logs"),
            lock_dir: base.join("locks"),
        }
    }

    /// Cria os diretórios com permissão `0700` (SEG-53).
    pub fn ensure(&self) -> CoreResult<()> {
        for dir in self.all() {
            ensure_private_dir(dir)?;
        }
        Ok(())
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn google_client_file(&self) -> PathBuf {
        self.config_dir.join("google-client.json")
    }

    /// Units do systemd do usuário (`~/.config/systemd/user`) — irmã de `config_dir`.
    /// Em [`AppPaths::in_dir`], fica dentro do diretório de teste.
    pub fn systemd_unit_dir(&self) -> PathBuf {
        self.config_dir
            .parent()
            .unwrap_or(&self.config_dir)
            .join("systemd")
            .join("user")
    }

    pub fn state_db(&self) -> PathBuf {
        self.data_dir.join("state.db")
    }

    /// Diretórios do próprio app: nunca podem ser fonte e são pulados no scan (RN-12).
    pub fn app_dirs(&self) -> Vec<PathBuf> {
        self.all().into_iter().cloned().collect()
    }

    fn all(&self) -> [&PathBuf; 4] {
        [
            &self.config_dir,
            &self.data_dir,
            &self.log_dir,
            &self.lock_dir,
        ]
    }
}
