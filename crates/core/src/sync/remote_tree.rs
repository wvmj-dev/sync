//! Estado atual do destino: listagem recursiva a partir da pasta da tarefa.
//! Fase 4: substituir por Changes API incremental quando o provedor suportar.

use std::collections::{BTreeMap, HashMap, VecDeque};

use crate::domain::rel_path::{join_str, validate_name};
use crate::domain::{EntryKind, RelPath, SkipReason};
use crate::error::{CoreError, CoreResult};
use crate::provider::{RemoteEntry, StorageProvider};

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

/// Resolve o destino sem criar nada e lista tudo abaixo dele.
pub async fn load_remote_tree(
    provider: &dyn StorageProvider,
    dest: &RelPath,
) -> CoreResult<RemoteTree> {
    let mut current = provider.app_root().await?.id;
    for part in dest.components() {
        let children = provider.list_children(&current).await?;
        let found: Vec<_> = children.into_iter().filter(|c| c.name == part).collect();
        match found.as_slice() {
            [] => return Ok(RemoteTree::default()),
            [one] if one.kind == EntryKind::Folder => current = one.id.clone(),
            [_] => {
                return Err(CoreError::Destination(format!(
                    "'{dest}' existe mas '{part}' não é uma pasta"
                )));
            }
            _ => {
                return Err(CoreError::Destination(format!(
                    "há mais de uma pasta '{part}' em '{dest}' — renomeie uma delas no provedor"
                )));
            }
        }
    }

    let mut tree = RemoteTree {
        dest_id: Some(current.clone()),
        ..RemoteTree::default()
    };
    // Sequencial por enquanto; paralelizar com limite quando o provedor Drive existir.
    let mut queue = VecDeque::from([(String::new(), current)]);
    while let Some((prefix, folder_id)) = queue.pop_front() {
        let children = provider.list_children(&folder_id).await?;
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
    Ok(tree)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::LocalProvider;

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
