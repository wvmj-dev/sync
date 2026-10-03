//! Provedor "pasta local" (HD externo, NAS montado). Também é o destino falso
//! usado nos testes de ponta a ponta — sem rede e sem conta Google.
//!
//! IDs são caminhos relativos à raiz (`""` = raiz), sempre validados (SEG-20).
//! Limitação conhecida: a listagem calcula o hash de cada arquivo; um cache
//! persistente fica para quando o destino local for usado em pastas grandes.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domain::EntryKind;
use crate::domain::rel_path::{RelPath, join_str, parent_str, validate_name};
use crate::fsutil::create_private_file;
use crate::hash::{HashKind, copy_hashing, hash_file};
use crate::provider::{ProviderError, ProviderResult, RemoteEntry, StorageProvider, StorageQuota};

/// Marcador que identifica a pasta como raiz do Syncly (RN-07). Se sumir — p.ex.
/// HD desmontado, restando só o ponto de montagem vazio — nada é gravado.
pub const ROOT_MARKER: &str = ".syncly-root";
const TRASH_DIR: &str = ".syncly-trash";
const RESERVED_PREFIX: &str = ".syncly-";
const HASH: HashKind = HashKind::Md5;

#[derive(Debug, Clone)]
pub struct LocalProvider {
    root: PathBuf,
}

impl LocalProvider {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Prepara uma pasta existente para ser raiz de uma conta local (RN-07).
    pub fn initialize(root: &Path) -> ProviderResult<()> {
        if !root.is_dir() {
            return Err(ProviderError::NotFound(root.display().to_string()));
        }
        let marker = root.join(ROOT_MARKER);
        if !marker.exists() {
            fs::write(
                &marker,
                b"Pasta gerenciada pelo Syncly. Nao apague este arquivo.\n",
            )?;
        }
        Ok(())
    }

    fn check_root(&self) -> ProviderResult<()> {
        if self.root.join(ROOT_MARKER).is_file() {
            Ok(())
        } else {
            Err(ProviderError::NotFound(format!(
                "pasta raiz '{}' (disco desmontado?)",
                self.root.display()
            )))
        }
    }

    /// Converte um ID em caminho absoluto, rejeitando travessia (SEG-20).
    fn resolve(&self, id: &str) -> ProviderResult<PathBuf> {
        let rel = RelPath::parse(id).map_err(ProviderError::BadRequest)?;
        if rel.components().any(|c| c.starts_with(RESERVED_PREFIX)) {
            return Err(ProviderError::BadRequest(format!(
                "caminho reservado: {id}"
            )));
        }
        Ok(rel
            .components()
            .fold(self.root.clone(), |acc, part| acc.join(part)))
    }

    fn entry(id: String, path: &Path, with_hash: bool) -> ProviderResult<RemoteEntry> {
        let meta = fs::symlink_metadata(path)?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let modified_at = meta.modified().ok().map(DateTime::<Utc>::from);
        if meta.is_dir() {
            return Ok(RemoteEntry {
                id,
                name,
                kind: EntryKind::Folder,
                size: None,
                hash: None,
                modified_at,
                modified_by: None,
            });
        }
        let hash = if with_hash {
            Some(hash_file(path, HASH)?)
        } else {
            None
        };
        Ok(RemoteEntry {
            id,
            name,
            kind: EntryKind::File,
            size: Some(meta.len()),
            hash,
            modified_at,
            modified_by: None,
        })
    }

    /// Copia para um temporário na pasta de destino e renomeia (SEG-14).
    fn write_file(dest: &Path, src: &Path) -> ProviderResult<String> {
        let dir = dest.parent().unwrap_or(Path::new("."));
        let tmp = dir.join(format!("{RESERVED_PREFIX}tmp-{}", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut file = create_private_file(&tmp)?;
            let (_, hash) = copy_hashing(src, &mut file, HASH)?;
            fs::rename(&tmp, dest)?;
            Ok(hash)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result.map_err(ProviderError::Io)
    }
}

/// Executa E/S bloqueante fora do runtime assíncrono.
async fn blocking<T, F>(f: F) -> ProviderResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> ProviderResult<T> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ProviderError::Other(format!("tarefa interrompida: {e}")))?
}

#[async_trait]
impl StorageProvider for LocalProvider {
    fn id(&self) -> &'static str {
        "local"
    }

    fn hash_kind(&self) -> HashKind {
        HASH
    }

    async fn app_root(&self) -> ProviderResult<RemoteEntry> {
        self.check_root()?;
        Self::entry(String::new(), &self.root, false)
    }

    async fn list_children(&self, folder_id: &str) -> ProviderResult<Vec<RemoteEntry>> {
        self.check_root()?;
        let dir = self.resolve(folder_id)?;
        let folder_id = folder_id.trim_matches('/').to_string();
        blocking(move || {
            let mut out = Vec::new();
            for item in fs::read_dir(&dir)? {
                let item = item?;
                let Some(name) = item.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                let file_type = item.file_type()?;
                if name.starts_with(RESERVED_PREFIX) || !(file_type.is_dir() || file_type.is_file())
                {
                    continue;
                }
                out.push(Self::entry(
                    join_str(&folder_id, &name),
                    &item.path(),
                    true,
                )?);
            }
            Ok(out)
        })
        .await
    }

    async fn get(&self, id: &str) -> ProviderResult<Option<RemoteEntry>> {
        let path = self.resolve(id)?;
        let id = id.trim_matches('/').to_string();
        blocking(move || match fs::symlink_metadata(&path) {
            Ok(_) => Self::entry(id, &path, true).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        })
        .await
    }

    async fn create_folder(&self, parent_id: &str, name: &str) -> ProviderResult<RemoteEntry> {
        self.check_root()?;
        validate_name(name).map_err(ProviderError::BadRequest)?;
        let path = self.resolve(parent_id)?.join(name);
        match fs::create_dir(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && path.is_dir() => {}
            Err(e) => return Err(e.into()),
        }
        Self::entry(join_str(parent_id.trim_matches('/'), name), &path, false)
    }

    async fn upload_new(
        &self,
        parent_id: &str,
        name: &str,
        local: &Path,
    ) -> ProviderResult<RemoteEntry> {
        self.check_root()?;
        validate_name(name).map_err(ProviderError::BadRequest)?;
        let dest = self.resolve(parent_id)?.join(name);
        if dest.exists() {
            return Err(ProviderError::AlreadyExists(name.to_string()));
        }
        let id = join_str(parent_id.trim_matches('/'), name);
        let src = local.to_path_buf();
        blocking(move || {
            let hash = Self::write_file(&dest, &src)?;
            let mut entry = Self::entry(id, &dest, false)?;
            entry.hash = Some(hash);
            Ok(entry)
        })
        .await
    }

    async fn upload_replace(&self, id: &str, local: &Path) -> ProviderResult<RemoteEntry> {
        self.check_root()?;
        let dest = self.resolve(id)?;
        if !dest.is_file() {
            return Err(ProviderError::NotFound(id.to_string()));
        }
        let id = id.trim_matches('/').to_string();
        let src = local.to_path_buf();
        blocking(move || {
            let hash = Self::write_file(&dest, &src)?;
            let mut entry = Self::entry(id, &dest, false)?;
            entry.hash = Some(hash);
            Ok(entry)
        })
        .await
    }

    async fn download(&self, id: &str, dest: &Path) -> ProviderResult<()> {
        self.check_root()?;
        let src = self.resolve(id)?;
        if !src.is_file() {
            return Err(ProviderError::NotFound(id.to_string()));
        }
        let dest = dest.to_path_buf();
        blocking(move || Self::write_file(&dest, &src).map(|_| ())).await
    }

    async fn rename(&self, id: &str, new_name: &str) -> ProviderResult<RemoteEntry> {
        self.check_root()?;
        validate_name(new_name).map_err(ProviderError::BadRequest)?;
        let from = self.resolve(id)?;
        let parent = parent_str(id.trim_matches('/'));
        let to = self.resolve(parent)?.join(new_name);
        if to.exists() {
            return Err(ProviderError::AlreadyExists(new_name.to_string()));
        }
        fs::rename(&from, &to)?;
        Self::entry(join_str(parent, new_name), &to, false)
    }

    async fn trash(&self, id: &str) -> ProviderResult<()> {
        self.check_root()?;
        let from = self.resolve(id)?;
        if id.trim_matches('/').is_empty() {
            return Err(ProviderError::BadRequest(
                "a raiz não pode ir para a lixeira".into(),
            ));
        }
        let name = from
            .file_name()
            .ok_or_else(|| ProviderError::BadRequest(id.to_string()))?
            .to_owned();
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        // RN-44: preserva o nome original dentro de uma pasta única na lixeira.
        let bucket = self
            .root
            .join(TRASH_DIR)
            .join(format!("{stamp}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&bucket)?;
        fs::rename(&from, bucket.join(name))?;
        Ok(())
    }

    async fn quota(&self) -> ProviderResult<Option<StorageQuota>> {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path, contents: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn provider() -> (tempfile::TempDir, LocalProvider) {
        let dir = tempfile::tempdir().unwrap();
        LocalProvider::initialize(dir.path()).unwrap();
        let p = LocalProvider::new(dir.path());
        (dir, p)
    }

    #[tokio::test]
    async fn rn07_sem_marcador_nada_e_gravado() {
        let dir = tempfile::tempdir().unwrap();
        let p = LocalProvider::new(dir.path());
        assert!(matches!(
            p.app_root().await,
            Err(ProviderError::NotFound(_))
        ));
        assert!(p.create_folder("", "x").await.is_err());
    }

    #[tokio::test]
    async fn seg20_ids_com_travessia_sao_rejeitados() {
        let (_d, p) = provider();
        assert!(matches!(
            p.list_children("../etc").await,
            Err(ProviderError::BadRequest(_))
        ));
        assert!(p.trash(".syncly-root").await.is_err());
    }

    #[tokio::test]
    async fn envia_lista_renomeia_e_manda_para_lixeira() {
        let (dir, p) = provider();
        let src_dir = tempfile::tempdir().unwrap();
        let src = src_dir.path().join("fonte.txt");
        touch(&src, b"abc");

        let folder = p.create_folder("", "Docs").await.unwrap();
        let up = p.upload_new(&folder.id, "a.txt", &src).await.unwrap();
        assert_eq!(up.id, "Docs/a.txt");
        assert_eq!(up.hash.as_deref(), Some("900150983cd24fb0d6963f7d28e17f72"));
        assert!(matches!(
            p.upload_new(&folder.id, "a.txt", &src).await,
            Err(ProviderError::AlreadyExists(_))
        ));

        let listed = p.list_children("Docs").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].hash, up.hash);

        let renamed = p.rename("Docs/a.txt", "b.txt").await.unwrap();
        assert_eq!(renamed.id, "Docs/b.txt");

        p.trash("Docs/b.txt").await.unwrap();
        assert!(p.list_children("Docs").await.unwrap().is_empty());
        // A lixeira não aparece nas listagens, mas o arquivo continua no disco.
        assert_eq!(p.list_children("").await.unwrap().len(), 1);
        let trash = dir.path().join(TRASH_DIR);
        let bucket = fs::read_dir(&trash)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(bucket.join("b.txt").is_file());
    }
}
