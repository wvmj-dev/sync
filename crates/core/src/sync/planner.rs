//! Planejador: função pura `(local, índice, remoto, modo) → Plano` (RN-30).
//! É a ÚNICA fonte de decisões de sincronização — a tabela de decisão está em
//! `docs/REGRAS_DE_NEGOCIO.md` §5 e cada linha tem um teste abaixo.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::domain::rel_path::{join_str, parent_str};
use crate::domain::{
    Action, EntryKind, IndexEntry, LocalEntry, Plan, PlanStats, SkipReason, SyncMode,
};
use crate::provider::RemoteEntry;

pub struct PlanInput<'a> {
    pub mode: SyncMode,
    pub local: &'a BTreeMap<String, LocalEntry>,
    pub index: &'a BTreeMap<String, IndexEntry>,
    pub remote: &'a BTreeMap<String, RemoteEntry>,
    /// Caminhos com estado incerto (leitura local falhou, nome remoto duplicado…).
    pub blocked: &'a BTreeMap<String, SkipReason>,
    /// RN-28: excluídos pelos filtros — deixam de ser acompanhados, nunca vão para a lixeira.
    pub excluded: &'a BTreeSet<String>,
    /// Carimbo usado nos nomes de conflito, ex.: `2026-10-03 14-30` (RN-35).
    pub conflict_stamp: &'a str,
}

#[derive(Default)]
struct Buckets {
    create: Vec<Action>,
    transfer: Vec<Action>,
    trash_files: Vec<Action>,
    folder_candidates: Vec<(String, String)>,
    forget: Vec<Action>,
    skip: Vec<Action>,
}

pub fn plan(input: &PlanInput<'_>) -> Plan {
    let paths: BTreeSet<&str> = input
        .local
        .keys()
        .chain(input.index.keys())
        .chain(input.remote.keys())
        .map(String::as_str)
        .collect();

    let mut blocked = input.blocked.clone();
    let mut taken = HashSet::new();
    let mut b = Buckets::default();

    // Ordem lexicográfica: um pai sempre vem antes dos seus descendentes.
    for path in paths {
        if let Some((root, reason)) = blocked_ancestor(&blocked, path) {
            if root == path {
                b.skip.push(skip(path, reason));
            }
            continue;
        }
        // RN-28
        if self_and_ancestors(path).any(|p| input.excluded.contains(p)) {
            if input.index.contains_key(path) {
                b.forget.push(Action::ForgetIndex { path: path.into() });
            }
            continue;
        }
        let local = input.local.get(path);
        let index = input.index.get(path);
        let remote = input.remote.get(path);

        match (local, remote) {
            // RN-33
            (Some(l), Some(r)) if l.kind != r.kind => {
                b.skip.push(skip(path, SkipReason::KindMismatch));
                blocked.insert(path.to_string(), SkipReason::KindMismatch);
            }
            (Some(l), Some(r)) => match l.kind {
                EntryKind::Folder => {
                    let known =
                        index.is_some_and(|i| i.kind == EntryKind::Folder && i.remote_id == r.id);
                    if !known {
                        b.transfer.push(Action::Adopt {
                            path: path.into(),
                            remote_id: r.id.clone(),
                        });
                    }
                }
                EntryKind::File => plan_file_both(input, &mut b, &mut taken, path, l, index, r),
            },
            // Novo — ou apagado no destino por fora: (re)envia.
            (Some(l), None) => match l.kind {
                EntryKind::Folder => b.create.push(Action::CreateFolder { path: path.into() }),
                EntryKind::File => b.transfer.push(Action::Upload {
                    path: path.into(),
                    size: l.size,
                }),
            },
            (None, Some(r)) => match index {
                // RN-31: item desconhecido — nunca tocar.
                None => {}
                Some(i) if i.kind != r.kind || i.remote_id != r.id => {
                    b.forget.push(Action::ForgetIndex { path: path.into() });
                }
                Some(i) => match input.mode {
                    SyncMode::Upload => b.forget.push(Action::ForgetIndex { path: path.into() }),
                    SyncMode::Mirror if r.kind == EntryKind::Folder => {
                        b.folder_candidates.push((path.to_string(), r.id.clone()));
                    }
                    SyncMode::Mirror if remote_unchanged(i, r) => {
                        b.trash_files.push(Action::Trash {
                            path: path.into(),
                            remote_id: r.id.clone(),
                            kind: EntryKind::File,
                        });
                    }
                    SyncMode::Mirror => {
                        b.skip
                            .push(skip(path, SkipReason::RemoteChangedAfterLocalDelete));
                        b.forget.push(Action::ForgetIndex { path: path.into() });
                    }
                },
            },
            (None, None) => {
                if index.is_some() {
                    b.forget.push(Action::ForgetIndex { path: path.into() });
                }
            }
        }
    }

    let trash_folders = plan_folder_trash(input, &b);
    let actions: Vec<Action> = [
        b.create,
        b.transfer,
        b.trash_files,
        trash_folders,
        b.forget,
        b.skip,
    ]
    .into_iter()
    .flatten()
    .collect();
    Plan {
        stats: stats(&actions),
        actions,
    }
}

/// Arquivo existe dos dois lados.
fn plan_file_both(
    input: &PlanInput<'_>,
    b: &mut Buckets,
    taken: &mut HashSet<String>,
    path: &str,
    l: &LocalEntry,
    index: Option<&IndexEntry>,
    r: &RemoteEntry,
) {
    let Some(remote_hash) = &r.hash else {
        b.skip.push(skip(path, SkipReason::RemoteWithoutChecksum));
        return;
    };
    let Some(local_hash) = &l.hash else {
        b.skip.push(skip(path, SkipReason::LocalUnreadable));
        return;
    };

    // Conteúdo igual: no máximo atualiza o índice (adoção, RN-56).
    if local_hash == remote_hash {
        let fresh = index.is_some_and(|i| {
            i.kind == EntryKind::File
                && i.remote_id == r.id
                && i.local_size == l.size
                && i.local_mtime_ns == l.mtime_ns
                && i.local_hash.as_ref() == Some(local_hash)
                && i.remote_hash.as_ref() == Some(remote_hash)
        });
        if !fresh {
            b.transfer.push(Action::Adopt {
                path: path.into(),
                remote_id: r.id.clone(),
            });
        }
        return;
    }

    if index.is_some_and(|i| remote_unchanged(i, r)) {
        b.transfer.push(Action::Update {
            path: path.into(),
            remote_id: r.id.clone(),
            size: l.size,
        });
    } else {
        // RN-35: conflito — mantém as duas versões.
        b.transfer.push(Action::ResolveConflict {
            path: path.into(),
            remote_id: r.id.clone(),
            conflict_name: conflict_name(input, taken, path),
            size: l.size,
        });
    }
}

/// O destino ainda é exatamente o que o app sincronizou da última vez?
fn remote_unchanged(i: &IndexEntry, r: &RemoteEntry) -> bool {
    i.kind == r.kind && i.remote_id == r.id && i.remote_hash.is_some() && i.remote_hash == r.hash
}

/// RN-32: pasta só vai para a lixeira se TODO o conteúdo remoto também for.
fn plan_folder_trash(input: &PlanInput<'_>, b: &Buckets) -> Vec<Action> {
    let mut trashed: HashSet<&str> = b.trash_files.iter().map(Action::path).collect();
    let mut candidates: Vec<_> = b.folder_candidates.iter().collect();
    // Mais profundas primeiro (RN-39).
    candidates.sort_by_key(|(p, _)| std::cmp::Reverse(p.matches('/').count()));

    let mut out = Vec::new();
    for (path, remote_id) in candidates {
        let prefix = format!("{path}/");
        let all_gone = input
            .remote
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .all(|(k, _)| trashed.contains(k.as_str()));
        if all_gone {
            trashed.insert(path);
            out.push(Action::Trash {
                path: path.clone(),
                remote_id: remote_id.clone(),
                kind: EntryKind::Folder,
            });
        }
    }
    out
}

/// `relatorio.docx` → `relatorio (conflito remoto 2026-10-03 14-30).docx`, único na pasta.
fn conflict_name(input: &PlanInput<'_>, taken: &mut HashSet<String>, path: &str) -> String {
    let parent = parent_str(path);
    let file = path.rsplit('/').next().unwrap_or(path);
    let (stem, ext) = match file.rfind('.') {
        Some(i) if i > 0 => file.split_at(i),
        _ => (file, ""),
    };
    let stem = truncate_bytes(stem, 180);
    for n in 1.. {
        let suffix = if n == 1 {
            String::new()
        } else {
            format!(" {n}")
        };
        let name = format!(
            "{stem} (conflito remoto {}{suffix}){ext}",
            input.conflict_stamp
        );
        let full = join_str(parent, &name);
        if !input.remote.contains_key(&full)
            && !input.local.contains_key(&full)
            && taken.insert(full)
        {
            return name;
        }
    }
    unreachable!("a sequência de sufixos é infinita")
}

fn truncate_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn blocked_ancestor<'a>(
    blocked: &'a BTreeMap<String, SkipReason>,
    path: &str,
) -> Option<(&'a str, SkipReason)> {
    self_and_ancestors(path).find_map(|p| blocked.get_key_value(p).map(|(k, r)| (k.as_str(), *r)))
}

/// O próprio caminho e seus ancestrais, do mais fundo ao mais raso (`a/b/c`, `a/b`, `a`).
fn self_and_ancestors(path: &str) -> impl Iterator<Item = &str> {
    std::iter::successors(Some(path), |p| Some(parent_str(p))).take_while(|p| !p.is_empty())
}

fn skip(path: &str, reason: SkipReason) -> Action {
    Action::Skip {
        path: path.into(),
        reason,
    }
}

fn stats(actions: &[Action]) -> PlanStats {
    let mut s = PlanStats::default();
    for a in actions {
        match a {
            Action::CreateFolder { .. } => s.folders_to_create += 1,
            Action::Upload { size, .. } => {
                s.uploads += 1;
                s.bytes_to_send += size;
            }
            Action::Update { size, .. } => {
                s.updates += 1;
                s.bytes_to_send += size;
            }
            Action::ResolveConflict { size, .. } => {
                s.conflicts += 1;
                s.bytes_to_send += size;
            }
            Action::Adopt { .. } => s.adopted += 1,
            Action::Trash {
                kind: EntryKind::File,
                ..
            } => s.trash_files += 1,
            Action::Trash { .. } => s.trash_folders += 1,
            Action::ForgetIndex { .. } => s.forgotten += 1,
            Action::Skip { .. } => s.skipped += 1,
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    const STAMP: &str = "2026-10-03 14-30";

    #[derive(Default)]
    struct World {
        local: BTreeMap<String, LocalEntry>,
        index: BTreeMap<String, IndexEntry>,
        remote: BTreeMap<String, RemoteEntry>,
        blocked: BTreeMap<String, SkipReason>,
        excluded: BTreeSet<String>,
    }

    impl World {
        fn local_file(mut self, path: &str, hash: &str) -> Self {
            self.local.insert(
                path.into(),
                LocalEntry {
                    kind: EntryKind::File,
                    size: 10,
                    mtime_ns: 1,
                    hash: Some(hash.into()),
                },
            );
            self
        }
        fn local_dir(mut self, path: &str) -> Self {
            self.local.insert(
                path.into(),
                LocalEntry {
                    kind: EntryKind::Folder,
                    size: 0,
                    mtime_ns: 1,
                    hash: None,
                },
            );
            self
        }
        fn remote_file(mut self, path: &str, hash: Option<&str>) -> Self {
            self.remote
                .insert(path.into(), remote(path, EntryKind::File, hash));
            self
        }
        fn remote_dir(mut self, path: &str) -> Self {
            self.remote
                .insert(path.into(), remote(path, EntryKind::Folder, None));
            self
        }
        /// Índice como se `path` tivesse sido sincronizado com `hash` dos dois lados.
        fn synced(mut self, path: &str, kind: EntryKind, hash: Option<&str>) -> Self {
            self.index.insert(
                path.into(),
                IndexEntry {
                    path: path.into(),
                    kind,
                    local_size: 10,
                    local_mtime_ns: 1,
                    local_hash: hash.map(Into::into),
                    remote_id: format!("id:{path}"),
                    remote_hash: hash.map(Into::into),
                    synced_at: Utc::now(),
                    remote_modified_at: None,
                    remote_modified_by: None,
                },
            );
            self
        }
        fn plan(&self, mode: SyncMode) -> Plan {
            plan(&PlanInput {
                mode,
                local: &self.local,
                index: &self.index,
                remote: &self.remote,
                blocked: &self.blocked,
                excluded: &self.excluded,
                conflict_stamp: STAMP,
            })
        }
    }

    fn remote(path: &str, kind: EntryKind, hash: Option<&str>) -> RemoteEntry {
        RemoteEntry {
            id: format!("id:{path}"),
            name: path.rsplit('/').next().unwrap().into(),
            kind,
            size: Some(10),
            hash: hash.map(Into::into),
            modified_at: None,
            modified_by: None,
        }
    }

    fn types(plan: &Plan) -> Vec<String> {
        plan.actions
            .iter()
            .map(|a| {
                let t = serde_json::to_value(a).unwrap()["type"]
                    .as_str()
                    .unwrap()
                    .to_string();
                format!("{t} {}", a.path())
            })
            .collect()
    }

    const F: EntryKind = EntryKind::File;
    const D: EntryKind = EntryKind::Folder;

    #[test]
    fn arquivo_novo_e_enviado() {
        let p = World::default()
            .local_dir("Docs")
            .local_file("Docs/a", "h1")
            .plan(SyncMode::Upload);
        assert_eq!(types(&p), ["create_folder Docs", "upload Docs/a"]);
        assert_eq!(p.stats.bytes_to_send, 10);
    }

    #[test]
    fn apagado_no_destino_por_fora_e_reenviado() {
        let p = World::default()
            .local_file("a", "h1")
            .synced("a", F, Some("h1"))
            .plan(SyncMode::Upload);
        assert_eq!(types(&p), ["upload a"]);
    }

    #[test]
    fn rn56_conteudo_igual_sem_indice_e_adotado() {
        let p = World::default()
            .local_file("a", "h1")
            .remote_file("a", Some("h1"))
            .plan(SyncMode::Upload);
        assert_eq!(types(&p), ["adopt a"]);
        assert!(!p.changes_remote());
    }

    #[test]
    fn tudo_sincronizado_gera_plano_vazio() {
        let p = World::default()
            .local_file("a", "h1")
            .remote_file("a", Some("h1"))
            .synced("a", F, Some("h1"))
            .plan(SyncMode::Mirror);
        assert!(p.actions.is_empty());
    }

    #[test]
    fn alterado_localmente_e_atualizado() {
        let p = World::default()
            .local_file("a", "h2")
            .remote_file("a", Some("h1"))
            .synced("a", F, Some("h1"))
            .plan(SyncMode::Upload);
        assert_eq!(types(&p), ["update a"]);
    }

    #[test]
    fn rn35_alterado_dos_dois_lados_e_conflito() {
        let p = World::default()
            .local_file("pasta/rel.final.docx", "h2")
            .remote_file("pasta/rel.final.docx", Some("h3"))
            .synced("pasta/rel.final.docx", F, Some("h1"))
            .plan(SyncMode::Upload);
        match &p.actions[0] {
            Action::ResolveConflict { conflict_name, .. } => {
                assert_eq!(
                    conflict_name,
                    "rel.final (conflito remoto 2026-10-03 14-30).docx"
                );
            }
            other => panic!("esperava conflito, veio {other:?}"),
        }
    }

    #[test]
    fn rn35_sem_indice_e_conteudo_diferente_e_conflito_com_nome_unico() {
        let p = World::default()
            .local_file("a.txt", "h1")
            .remote_file("a.txt", Some("h2"))
            .remote_file("a (conflito remoto 2026-10-03 14-30).txt", Some("h9"))
            .plan(SyncMode::Upload);
        match &p.actions[0] {
            Action::ResolveConflict { conflict_name, .. } => {
                assert_eq!(conflict_name, "a (conflito remoto 2026-10-03 14-30 2).txt");
            }
            other => panic!("esperava conflito, veio {other:?}"),
        }
    }

    #[test]
    fn apagado_localmente_no_modo_envio_so_esquece() {
        let p = World::default()
            .remote_file("a", Some("h1"))
            .synced("a", F, Some("h1"))
            .plan(SyncMode::Upload);
        assert_eq!(types(&p), ["forget_index a"]);
        assert_eq!(p.stats.trash_files, 0);
    }

    #[test]
    fn apagado_localmente_no_espelho_vai_para_lixeira() {
        let p = World::default()
            .remote_file("a", Some("h1"))
            .synced("a", F, Some("h1"))
            .plan(SyncMode::Mirror);
        assert_eq!(types(&p), ["trash a"]);
    }

    #[test]
    fn apagado_localmente_mas_alterado_no_destino_e_mantido() {
        let p = World::default()
            .remote_file("a", Some("h2"))
            .synced("a", F, Some("h1"))
            .plan(SyncMode::Mirror);
        assert_eq!(types(&p), ["forget_index a", "skip a"]);
        assert_eq!(p.stats.trash_files, 0);
    }

    #[test]
    fn rn31_item_remoto_desconhecido_nunca_e_tocado() {
        for mode in [SyncMode::Upload, SyncMode::Mirror] {
            let p = World::default()
                .remote_file("estranho", Some("h"))
                .remote_dir("pasta")
                .plan(mode);
            assert!(p.actions.is_empty());
        }
    }

    #[test]
    fn rn33_tipos_divergentes_bloqueiam_o_caminho() {
        let p = World::default()
            .local_dir("a")
            .local_file("a/x", "h1")
            .remote_file("a", Some("h"))
            .plan(SyncMode::Upload);
        assert_eq!(types(&p), ["skip a"]);
    }

    #[test]
    fn rn34_sem_checksum_e_ignorado() {
        let p = World::default()
            .local_file("doc", "h1")
            .remote_file("doc", None)
            .plan(SyncMode::Upload);
        assert_eq!(types(&p), ["skip doc"]);
    }

    #[test]
    fn caminho_bloqueado_nao_gera_lixeira_abaixo_dele() {
        let mut w = World::default()
            .remote_dir("d")
            .synced("d", D, None)
            .remote_file("d/x", Some("h1"))
            .synced("d/x", F, Some("h1"));
        w.blocked.insert("d".into(), SkipReason::LocalUnreadable);
        let p = w.plan(SyncMode::Mirror);
        assert_eq!(types(&p), ["skip d"]);
    }

    #[test]
    fn rn32_pasta_com_conteudo_desconhecido_nao_vai_para_lixeira() {
        let p = World::default()
            .remote_dir("d")
            .synced("d", D, None)
            .remote_file("d/x", Some("h1"))
            .synced("d/x", F, Some("h1"))
            .remote_file("d/de-outro-app", Some("h9"))
            .plan(SyncMode::Mirror);
        assert_eq!(types(&p), ["trash d/x"]);
    }

    #[test]
    fn rn32_rn39_pasta_esvaziada_vai_para_lixeira_depois_dos_arquivos() {
        let p = World::default()
            .remote_dir("d")
            .synced("d", D, None)
            .remote_dir("d/e")
            .synced("d/e", D, None)
            .remote_file("d/e/x", Some("h1"))
            .synced("d/e/x", F, Some("h1"))
            .plan(SyncMode::Mirror);
        assert_eq!(types(&p), ["trash d/e/x", "trash d/e", "trash d"]);
        assert_eq!(p.stats.trash_folders, 2);
    }

    #[test]
    fn rn28_excluido_depois_de_sincronizado_nunca_vai_para_lixeira() {
        let mut w = World::default()
            .local_dir("d")
            .remote_dir("d")
            .synced("d", D, None)
            .remote_dir("d/node_modules")
            .synced("d/node_modules", D, None)
            .remote_file("d/node_modules/x.js", Some("h1"))
            .synced("d/node_modules/x.js", F, Some("h1"));
        w.excluded.insert("d/node_modules".into());
        let p = w.plan(SyncMode::Mirror);
        assert_eq!(
            types(&p),
            [
                "forget_index d/node_modules",
                "forget_index d/node_modules/x.js"
            ]
        );
        assert_eq!(p.stats.trash_files + p.stats.trash_folders, 0);
    }

    #[test]
    fn pasta_existente_dos_dois_lados_e_adotada_uma_vez() {
        let w = World::default().local_dir("d").remote_dir("d");
        assert_eq!(types(&w.plan(SyncMode::Upload)), ["adopt d"]);
        let w = w.synced("d", D, None);
        assert!(w.plan(SyncMode::Upload).actions.is_empty());
    }
}
