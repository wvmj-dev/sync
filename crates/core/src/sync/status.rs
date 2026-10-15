//! Status de uma tarefa arquivo por arquivo (RN-101 a RN-103): a situação de cada item,
//! comparando o computador, o índice (o último sync) e, se houver, a última "foto" do
//! destino. Sem foto, tudo vem do computador e do índice — funciona sem rede. Função pura.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::domain::rel_path::parent_str;
use crate::domain::{EntryKind, IndexEntry, LocalEntry};
use crate::provider::RemoteEntry;

/// Um item do destino na última "Atualizar do destino" (RN-103).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteInfo {
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub hash: Option<String>,
    pub modified_at: Option<DateTime<Utc>>,
    pub modified_by: Option<String>,
}

impl From<&RemoteEntry> for RemoteInfo {
    fn from(e: &RemoteEntry) -> Self {
        Self {
            kind: e.kind,
            size: e.size,
            hash: e.hash.clone(),
            modified_at: e.modified_at,
            modified_by: e.modified_by.clone(),
        }
    }
}

/// A "foto" do destino: caminho relativo à pasta da tarefa → item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSnapshot {
    pub fetched_at: DateTime<Utc>,
    pub entries: BTreeMap<String, RemoteInfo>,
}

/// RN-101. A ordem vai do mais tranquilo ao que mais pede atenção — uma pasta mostra a
/// pior situação entre os arquivos dentro dela.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Situation {
    Ignored,
    Synced,
    Pending,
    RemotelyChanged,
    LocallyChanged,
    Conflict,
}

impl Situation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ignored => "ignorado",
            Self::Synced => "sincronizado",
            Self::Pending => "pendente",
            Self::RemotelyChanged => "alterado no destino",
            Self::LocallyChanged => "alterado localmente",
            Self::Conflict => "conflito",
        }
    }
}

/// Situação de um arquivo ou pasta. Para pastas, datas e totais vêm dos arquivos dentro
/// dela (RN-102).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemStatus {
    pub path: String,
    pub name: String,
    pub kind: EntryKind,
    pub situation: Situation,
    /// Explica a situação ("novo", "apagado no destino"…).
    pub detail: Option<&'static str>,
    /// Arquivo: tamanho; pasta: soma dos arquivos dentro.
    pub size: u64,
    /// Arquivos dentro (pasta) — 1 para um arquivo.
    pub files: u64,
    pub local_modified: Option<DateTime<Utc>>,
    pub remote_modified: Option<DateTime<Utc>>,
    pub remote_modified_by: Option<String>,
    pub synced_at: Option<DateTime<Utc>>,
}

/// Quantos arquivos em cada situação.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SituationCounts {
    pub synced: u64,
    pub pending: u64,
    pub locally_changed: u64,
    pub remotely_changed: u64,
    pub conflict: u64,
    pub ignored: u64,
}

impl SituationCounts {
    fn add(&mut self, situation: Situation) {
        let slot = match situation {
            Situation::Synced => &mut self.synced,
            Situation::Pending => &mut self.pending,
            Situation::LocallyChanged => &mut self.locally_changed,
            Situation::RemotelyChanged => &mut self.remotely_changed,
            Situation::Conflict => &mut self.conflict,
            Situation::Ignored => &mut self.ignored,
        };
        *slot += 1;
    }
}

pub struct StatusInput<'a> {
    /// Varredura do computador (sem hash: tamanho e data bastam).
    pub local: &'a BTreeMap<String, LocalEntry>,
    pub index: &'a BTreeMap<String, IndexEntry>,
    pub snapshot: Option<&'a RemoteSnapshot>,
    /// RN-27, RN-28: ilegíveis e excluídos pelos filtros → (tipo, motivo).
    pub ignored: &'a BTreeMap<String, (EntryKind, &'static str)>,
}

/// O que se sabe do destino para um caminho: `None` = nada além do índice.
fn remote_view<'a>(input: &StatusInput<'a>, path: &str) -> Option<Option<&'a RemoteInfo>> {
    let snapshot = input.snapshot?;
    // RN-103: um sync depois da foto é informação mais nova — vale o índice.
    if input
        .index
        .get(path)
        .is_some_and(|i| i.synced_at > snapshot.fetched_at)
    {
        return None;
    }
    Some(snapshot.entries.get(path))
}

fn timestamp(ns: i64) -> Option<DateTime<Utc>> {
    let secs = ns.div_euclid(1_000_000_000);
    let nanos = u32::try_from(ns.rem_euclid(1_000_000_000)).ok()?;
    if ns <= 0 {
        return None;
    }
    DateTime::from_timestamp(secs, nanos)
}

/// Situação de um item olhando só para ele (pastas ainda sem agregar).
fn own_situation(
    input: &StatusInput<'_>,
    path: &str,
    kind: EntryKind,
) -> (Situation, Option<&'static str>) {
    if let Some((_, reason)) = input.ignored.get(path) {
        return (Situation::Ignored, Some(reason));
    }
    let local = input.local.get(path);
    let index = input.index.get(path);
    let remote = remote_view(input, path);
    let folder = kind == EntryKind::Folder;
    let local_changed = matches!((local, index), (Some(l), Some(i)) if !folder
        && (l.size != i.local_size || l.mtime_ns != i.local_mtime_ns));
    let remote_gone = index.is_some() && matches!(remote, Some(None));
    let remote_changed = remote_gone
        || matches!((index, remote), (Some(i), Some(Some(r))) if !folder
            && matches!((&i.remote_hash, &r.hash), (Some(a), Some(b)) if a != b));
    match (local, index) {
        (Some(_), Some(_)) if local_changed && remote_changed => (Situation::Conflict, None),
        (Some(_), Some(_)) if local_changed => {
            (Situation::LocallyChanged, Some("alterado no computador"))
        }
        (Some(_), Some(_)) if remote_gone => {
            (Situation::RemotelyChanged, Some("apagado no destino"))
        }
        (Some(_), Some(_)) if remote_changed => {
            (Situation::RemotelyChanged, Some("alterado no destino"))
        }
        (Some(_), Some(_)) => (Situation::Synced, None),
        (Some(_), None) if matches!(remote, Some(Some(_))) => {
            (Situation::Pending, Some("também existe no destino"))
        }
        (Some(_), None) => (Situation::Pending, Some("novo")),
        (None, Some(_)) => (Situation::LocallyChanged, Some("apagado no computador")),
        (None, None) => (Situation::RemotelyChanged, Some("só no destino")),
    }
}

/// RN-101, RN-102: a situação de cada item da tarefa (chave: caminho relativo ao destino).
pub fn compute(input: &StatusInput<'_>) -> BTreeMap<String, ItemStatus> {
    let mut paths: BTreeSet<&str> = BTreeSet::new();
    paths.extend(input.local.keys().map(String::as_str));
    paths.extend(input.index.keys().map(String::as_str));
    paths.extend(input.ignored.keys().map(String::as_str));
    if let Some(s) = input.snapshot {
        paths.extend(s.entries.keys().map(String::as_str));
    }

    let mut out: BTreeMap<String, ItemStatus> = BTreeMap::new();
    for path in paths {
        let local = input.local.get(path);
        let index = input.index.get(path);
        let remote = remote_view(input, path).flatten();
        let kind = local
            .map(|l| l.kind)
            .or(index.map(|i| i.kind))
            .or(remote.map(|r| r.kind))
            .or(input.ignored.get(path).map(|(k, _)| *k))
            .unwrap_or(EntryKind::File);
        let (situation, detail) = own_situation(input, path, kind);
        let file = kind == EntryKind::File;
        let (remote_modified, remote_modified_by) = match remote {
            Some(r) => (r.modified_at, r.modified_by.clone()),
            None => index.map_or((None, None), |i| {
                (i.remote_modified_at, i.remote_modified_by.clone())
            }),
        };
        out.insert(
            path.to_string(),
            ItemStatus {
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or(path).to_string(),
                kind,
                situation,
                detail,
                size: if file {
                    local
                        .map(|l| l.size)
                        .or(index.map(|i| i.local_size))
                        .or(remote.and_then(|r| r.size))
                        .unwrap_or(0)
                } else {
                    0
                },
                files: u64::from(file && situation != Situation::Ignored),
                local_modified: local.and_then(|l| timestamp(l.mtime_ns)),
                remote_modified,
                remote_modified_by,
                synced_at: index.map(|i| i.synced_at),
            },
        );
    }
    aggregate_folders(&mut out);
    out
}

/// RN-102: a pasta mostra a modificação mais recente entre os arquivos dentro dela, a soma
/// deles e a pior situação. Arquivos ignorados não pesam na situação da pasta.
fn aggregate_folders(out: &mut BTreeMap<String, ItemStatus>) {
    let files: Vec<ItemStatus> = out
        .values()
        .filter(|i| i.kind == EntryKind::File && i.situation != Situation::Ignored)
        .cloned()
        .collect();
    let mut seen_local: BTreeSet<String> = BTreeSet::new();
    for file in &files {
        let mut parent = parent_str(&file.path);
        while !parent.is_empty() {
            let folder = out.entry(parent.to_string()).or_insert_with(|| ItemStatus {
                path: parent.to_string(),
                name: parent.rsplit('/').next().unwrap_or(parent).to_string(),
                kind: EntryKind::Folder,
                situation: Situation::Synced,
                detail: None,
                size: 0,
                files: 0,
                local_modified: None,
                remote_modified: None,
                remote_modified_by: None,
                synced_at: None,
            });
            if folder.situation != Situation::Ignored {
                folder.situation = folder.situation.max(file.situation);
            }
            folder.size += file.size;
            folder.files += 1;
            // A data da própria pasta só conta se ela não tiver arquivos (RN-102).
            if seen_local.insert(folder.path.clone()) {
                folder.local_modified = None;
            }
            folder.local_modified = folder.local_modified.max(file.local_modified);
            if file.remote_modified > folder.remote_modified {
                folder.remote_modified = file.remote_modified;
                folder
                    .remote_modified_by
                    .clone_from(&file.remote_modified_by);
            }
            folder.synced_at = folder.synced_at.max(file.synced_at);
            parent = parent_str(parent);
        }
    }
}

/// Itens diretamente dentro de `folder` (`""` = raiz): pastas primeiro, depois por nome.
pub fn children<'a>(all: &'a BTreeMap<String, ItemStatus>, folder: &str) -> Vec<&'a ItemStatus> {
    let mut items: Vec<&ItemStatus> = all
        .values()
        .filter(|i| parent_str(&i.path) == folder && i.path != folder)
        .collect();
    items.sort_by(|a, b| {
        (a.kind != EntryKind::Folder, a.name.to_lowercase())
            .cmp(&(b.kind != EntryKind::Folder, b.name.to_lowercase()))
    });
    items
}

/// Quantos arquivos em cada situação, na tarefa inteira.
pub fn counts(all: &BTreeMap<String, ItemStatus>) -> SituationCounts {
    let mut counts = SituationCounts::default();
    for item in all.values().filter(|i| i.kind == EntryKind::File) {
        counts.add(item.situation);
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    const T0: i64 = 1_700_000_000_000_000_000;

    fn local(kind: EntryKind, size: u64, mtime_ns: i64) -> LocalEntry {
        LocalEntry {
            kind,
            size,
            mtime_ns,
            hash: None,
        }
    }

    fn indexed(path: &str, kind: EntryKind, size: u64, hash: &str) -> IndexEntry {
        IndexEntry {
            path: path.into(),
            kind,
            local_size: size,
            local_mtime_ns: T0,
            local_hash: Some(hash.into()),
            remote_id: format!("id-{path}"),
            remote_hash: Some(hash.into()),
            synced_at: Utc::now() - Duration::hours(1),
            remote_modified_at: Some(Utc::now() - Duration::hours(1)),
            remote_modified_by: Some("Eu".into()),
        }
    }

    fn remote(hash: &str, by: &str) -> RemoteInfo {
        RemoteInfo {
            kind: EntryKind::File,
            size: Some(1),
            hash: Some(hash.into()),
            modified_at: Some(Utc::now()),
            modified_by: Some(by.into()),
        }
    }

    /// Docs/ com: igual, alterado aqui, novo, apagado aqui; e um excluído.
    fn base() -> (BTreeMap<String, LocalEntry>, BTreeMap<String, IndexEntry>) {
        let mut l = BTreeMap::new();
        let mut i = BTreeMap::new();
        l.insert("Docs".into(), local(EntryKind::Folder, 0, T0));
        i.insert("Docs".into(), indexed("Docs", EntryKind::Folder, 0, ""));
        l.insert("Docs/igual.txt".into(), local(EntryKind::File, 1, T0));
        i.insert(
            "Docs/igual.txt".into(),
            indexed("Docs/igual.txt", EntryKind::File, 1, "a"),
        );
        l.insert("Docs/mudou.txt".into(), local(EntryKind::File, 5, T0 + 1));
        i.insert(
            "Docs/mudou.txt".into(),
            indexed("Docs/mudou.txt", EntryKind::File, 1, "b"),
        );
        l.insert("Docs/novo.txt".into(), local(EntryKind::File, 2, T0 + 9));
        i.insert(
            "Docs/sumiu.txt".into(),
            indexed("Docs/sumiu.txt", EntryKind::File, 1, "c"),
        );
        (l, i)
    }

    fn ignored() -> BTreeMap<String, (EntryKind, &'static str)> {
        let mut m = BTreeMap::new();
        m.insert(
            "Docs/node_modules".into(),
            (EntryKind::Folder, "excluído pelos filtros"),
        );
        m
    }

    #[test]
    fn rn101_rn103_situacao_sem_rede() {
        let (l, i) = base();
        let ign = ignored();
        let all = compute(&StatusInput {
            local: &l,
            index: &i,
            snapshot: None,
            ignored: &ign,
        });
        let s = |p: &str| (all[p].situation, all[p].detail);
        assert_eq!(s("Docs/igual.txt"), (Situation::Synced, None));
        assert_eq!(
            s("Docs/mudou.txt"),
            (Situation::LocallyChanged, Some("alterado no computador"))
        );
        assert_eq!(s("Docs/novo.txt"), (Situation::Pending, Some("novo")));
        assert_eq!(
            s("Docs/sumiu.txt"),
            (Situation::LocallyChanged, Some("apagado no computador"))
        );
        assert_eq!(s("Docs/node_modules").0, Situation::Ignored);
        // Sem foto do destino, a modificação no destino vem do último sync.
        assert_eq!(
            all["Docs/igual.txt"].remote_modified_by.as_deref(),
            Some("Eu")
        );

        let c = counts(&all);
        assert_eq!((c.synced, c.pending, c.locally_changed), (1, 1, 2));
        let kids: Vec<_> = children(&all, "Docs")
            .iter()
            .map(|i| i.name.clone())
            .collect();
        assert_eq!(
            kids,
            [
                "node_modules",
                "igual.txt",
                "mudou.txt",
                "novo.txt",
                "sumiu.txt"
            ]
        );
    }

    #[test]
    fn rn102_pasta_resume_os_arquivos_dentro() {
        let (l, i) = base();
        let ign = ignored();
        let all = compute(&StatusInput {
            local: &l,
            index: &i,
            snapshot: None,
            ignored: &ign,
        });
        let docs = &all["Docs"];
        // Pior situação entre os arquivos; o excluído não pesa.
        assert_eq!(docs.situation, Situation::LocallyChanged);
        assert_eq!(docs.files, 4);
        assert_eq!(docs.size, 1 + 5 + 2 + 1);
        // Modificação mais recente entre os arquivos dentro (novo.txt).
        assert_eq!(docs.local_modified, timestamp(T0 + 9));
    }

    #[test]
    fn rn101_foto_do_destino_mostra_mudancas_e_conflitos() {
        let (l, i) = base();
        let mut entries = BTreeMap::new();
        entries.insert("Docs/igual.txt".into(), remote("outro", "Ana"));
        entries.insert("Docs/mudou.txt".into(), remote("outro", "Ana"));
        entries.insert("Docs/extra.txt".into(), remote("x", "Ana"));
        // sumiu.txt não está mais no destino; novo.txt também não.
        let snapshot = RemoteSnapshot {
            fetched_at: Utc::now(),
            entries,
        };
        let ign = BTreeMap::new();
        let all = compute(&StatusInput {
            local: &l,
            index: &i,
            snapshot: Some(&snapshot),
            ignored: &ign,
        });
        let s = |p: &str| (all[p].situation, all[p].detail);
        assert_eq!(
            s("Docs/igual.txt"),
            (Situation::RemotelyChanged, Some("alterado no destino"))
        );
        assert_eq!(
            all["Docs/igual.txt"].remote_modified_by.as_deref(),
            Some("Ana")
        );
        assert_eq!(s("Docs/mudou.txt").0, Situation::Conflict);
        assert_eq!(
            s("Docs/extra.txt"),
            (Situation::RemotelyChanged, Some("só no destino"))
        );
        assert_eq!(s("Docs/novo.txt"), (Situation::Pending, Some("novo")));
        assert_eq!(all["Docs"].situation, Situation::Conflict);
    }

    #[test]
    fn rn103_sync_depois_da_foto_vale_o_indice() {
        let (l, mut i) = base();
        let snapshot = RemoteSnapshot {
            fetched_at: Utc::now() - Duration::hours(2),
            entries: BTreeMap::new(),
        };
        // igual.txt foi sincronizado depois da foto (que não o tinha).
        if let Some(e) = i.get_mut("Docs/igual.txt") {
            e.synced_at = Utc::now();
        }
        let ign = BTreeMap::new();
        let all = compute(&StatusInput {
            local: &l,
            index: &i,
            snapshot: Some(&snapshot),
            ignored: &ign,
        });
        assert_eq!(all["Docs/igual.txt"].situation, Situation::Synced);
        // mudou.txt também foi sincronizado depois da foto: só a mudança local conta.
        assert_eq!(all["Docs/mudou.txt"].situation, Situation::LocallyChanged);
        // novo.txt nunca foi sincronizado: a foto (sem ele) vale.
        assert_eq!(all["Docs/novo.txt"].situation, Situation::Pending);
    }
}
