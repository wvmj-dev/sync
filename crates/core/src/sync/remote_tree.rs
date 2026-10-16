//! Estado atual do destino: listagem recursiva a partir da pasta da tarefa.
//! Fase 4: substituir por Changes API incremental quando o provedor suportar.

use std::collections::{BTreeMap, HashMap, VecDeque};

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;

use crate::domain::rel_path::{join_str, validate_name};
use crate::domain::{EntryKind, RelPath, SkipReason};
use crate::error::{CoreError, CoreResult};
use crate::provider::{RemoteEntry, StorageProvider};

/// Pastas listadas ao mesmo tempo. O limitador do provedor (SEG-41) continua valendo: isso só
/// esconde a espera da rede, não faz mais requisições por segundo.
pub const PARALLEL_LISTINGS: usize = 4;

#[derive(Debug, Default)]
pub struct RemoteTree {
    /// `None` = a pasta de destino ainda não existe (o executor cria).
    pub dest_id: Option<String>,
    /// Chave: caminho relativo à pasta de destino.
    pub entries: BTreeMap<String, RemoteEntry>,
    /// RN-34: nomes duplicados — nada abaixo deles é decidido.
    pub blocked: BTreeMap<String, SkipReason>,
    /// RN-34: nomes impossíveis localmente (ex.: contém '/'); ignorados.
    pub invalid_names: Vec<String>,
}

/// Caminha da raiz do app até `path`. Com `create`, cria as pastas que faltarem; sem,
/// retorna `None` se algo não existir. Nomes duplicados → erro: o app nunca adivinha (RN-34).
pub async fn resolve_path(
    provider: &dyn StorageProvider,
    path: &RelPath,
    create: bool,
) -> CoreResult<Option<RemoteEntry>> {
    let mut current = provider.app_root().await?;
    for part in path.components() {
        let children = provider.list_children(&current.id).await?;
        let found: Vec<_> = children.into_iter().filter(|c| c.name == part).collect();
        current = match found.as_slice() {
            [] if create => provider.create_folder(&current.id, part).await?,
            [] => return Ok(None),
            [one] if one.kind == EntryKind::Folder => one.clone(),
            [_] => {
                return Err(CoreError::Destination(format!(
                    "'{path}' existe mas '{part}' não é uma pasta"
                )));
            }
            _ => {
                return Err(CoreError::Destination(format!(
                    "há mais de um item '{part}' em '{path}' — renomeie no provedor"
                )));
            }
        };
    }
    Ok(Some(current))
}

/// Resolve o destino sem criar nada e lista tudo abaixo dele.
pub async fn load_remote_tree(
    provider: &dyn StorageProvider,
    dest: &RelPath,
) -> CoreResult<RemoteTree> {
    let Some(dest_entry) = resolve_path(provider, dest, false).await? else {
        return Ok(RemoteTree::default());
    };
    let current = dest_entry.id;

    let mut tree = RemoteTree {
        dest_id: Some(current.clone()),
        ..RemoteTree::default()
    };
    // Até PARALLEL_LISTINGS pastas ao mesmo tempo; o primeiro erro interrompe tudo.
    let mut queue = VecDeque::from([(String::new(), current)]);
    let mut listing = FuturesUnordered::new();
    loop {
        while listing.len() < PARALLEL_LISTINGS
            && let Some((prefix, folder_id)) = queue.pop_front()
        {
            listing.push(async move {
                let children = provider.list_children(&folder_id).await;
                (prefix, children)
            });
        }
        let Some((prefix, children)) = listing.next().await else {
            break;
        };
        let children = children?;
        let mut counts: HashMap<String, usize> = HashMap::new();
        for child in &children {
            *counts.entry(child.name.clone()).or_default() += 1;
        }
        for child in children {
            let rel = join_str(&prefix, &child.name);
            if validate_name(&child.name).is_err() {
                tree.invalid_names.push(rel);
                continue;
            }
            if counts[&child.name] > 1 {
                tree.blocked.insert(rel, SkipReason::DuplicateRemoteName);
                continue;
            }
            if child.kind == EntryKind::Folder {
                queue.push_back((rel.clone(), child.id.clone()));
            }
            tree.entries.insert(rel, child);
        }
    }
    // A ordem de chegada varia com o paralelismo; o resultado, não.
    tree.invalid_names.sort();
    Ok(tree)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{HashKind, ProviderResult, StorageQuota};
    use crate::providers::LocalProvider;
    use async_trait::async_trait;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[tokio::test]
    async fn lista_recursivamente_a_partir_do_destino() {
        let tmp = tempfile::tempdir().unwrap();
        LocalProvider::initialize(tmp.path()).unwrap();
        std::fs::create_dir_all(tmp.path().join("Backup/Docs/sub")).unwrap();
        std::fs::write(tmp.path().join("Backup/Docs/sub/a.txt"), "a").unwrap();
        std::fs::write(tmp.path().join("fora.txt"), "x").unwrap();

        let p = LocalProvider::new(tmp.path());
        let tree = load_remote_tree(&p, &RelPath::parse("Backup").unwrap())
            .await
            .unwrap();
        assert_eq!(tree.dest_id.as_deref(), Some("Backup"));
        let keys: Vec<_> = tree.entries.keys().map(String::as_str).collect();
        assert_eq!(keys, ["Docs", "Docs/sub", "Docs/sub/a.txt"]);
        assert!(tree.entries["Docs/sub/a.txt"].hash.is_some());
    }

    /// Provedor local lento que mede quantas listagens correm ao mesmo tempo.
    struct Slow {
        inner: LocalProvider,
        now: AtomicUsize,
        peak: AtomicUsize,
    }

    #[async_trait]
    impl StorageProvider for Slow {
        fn id(&self) -> &'static str {
            "slow"
        }
        fn hash_kind(&self) -> HashKind {
            HashKind::Md5
        }
        async fn app_root(&self) -> ProviderResult<RemoteEntry> {
            self.inner.app_root().await
        }
        async fn list_children(&self, f: &str) -> ProviderResult<Vec<RemoteEntry>> {
            let now = self.now.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            let out = self.inner.list_children(f).await;
            self.now.fetch_sub(1, Ordering::SeqCst);
            out
        }
        async fn get(&self, id: &str) -> ProviderResult<Option<RemoteEntry>> {
            self.inner.get(id).await
        }
        async fn create_folder(&self, p: &str, n: &str) -> ProviderResult<RemoteEntry> {
            self.inner.create_folder(p, n).await
        }
        async fn upload_new(&self, p: &str, n: &str, l: &Path) -> ProviderResult<RemoteEntry> {
            self.inner.upload_new(p, n, l).await
        }
        async fn upload_replace(&self, id: &str, l: &Path) -> ProviderResult<RemoteEntry> {
            self.inner.upload_replace(id, l).await
        }
        async fn download(&self, id: &str, d: &Path) -> ProviderResult<()> {
            self.inner.download(id, d).await
        }
        async fn rename(&self, id: &str, n: &str) -> ProviderResult<RemoteEntry> {
            self.inner.rename(id, n).await
        }
        async fn trash(&self, id: &str) -> ProviderResult<()> {
            self.inner.trash(id).await
        }
        async fn quota(&self) -> ProviderResult<Option<StorageQuota>> {
            Ok(None)
        }
    }

    #[tokio::test]
    async fn lista_varias_pastas_ao_mesmo_tempo_com_limite() {
        let tmp = tempfile::tempdir().unwrap();
        LocalProvider::initialize(tmp.path()).unwrap();
        for i in 0..10 {
            let dir = tmp.path().join(format!("Backup/p{i}/sub"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("a.txt"), format!("{i}")).unwrap();
        }
        let slow = Slow {
            inner: LocalProvider::new(tmp.path()),
            now: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        };
        let dest = RelPath::parse("Backup").unwrap();
        let tree = load_remote_tree(&slow, &dest).await.unwrap();
        // Mesmo resultado da listagem simples, com paralelismo de verdade e dentro do limite.
        let plain = load_remote_tree(&LocalProvider::new(tmp.path()), &dest)
            .await
            .unwrap();
        assert_eq!(tree.entries.len(), 30);
        assert_eq!(
            tree.entries.keys().collect::<Vec<_>>(),
            plain.entries.keys().collect::<Vec<_>>()
        );
        let peak = slow.peak.load(Ordering::SeqCst);
        assert!(
            (2..=PARALLEL_LISTINGS).contains(&peak),
            "pico de {peak} listagens"
        );
    }

    #[tokio::test]
    async fn destino_inexistente_e_arvore_vazia() {
        let tmp = tempfile::tempdir().unwrap();
        LocalProvider::initialize(tmp.path()).unwrap();
        let p = LocalProvider::new(tmp.path());
        let tree = load_remote_tree(&p, &RelPath::parse("Nova/Pasta").unwrap())
            .await
            .unwrap();
        assert!(tree.dest_id.is_none() && tree.entries.is_empty());
    }
}
