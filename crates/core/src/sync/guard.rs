//! Guardas: bloqueiam planos perigosos antes de qualquer alteração (RN-36, RN-40 a RN-43).

use std::collections::BTreeMap;

use serde::Serialize;

use super::scanner::SourceStatus;
use crate::domain::rel_path::is_within_str;
use crate::domain::{EntryKind, GuardLimits, IndexEntry, Plan};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "verdict", content = "reasons", rename_all = "snake_case")]
pub enum GuardVerdict {
    Proceed,
    /// Só roda com confirmação explícita; execuções agendadas nunca confirmam (RN-43).
    NeedsConfirmation(Vec<String>),
    /// Não roda de jeito nenhum.
    Abort(Vec<String>),
}

pub struct GuardInput<'a> {
    pub sources: &'a [SourceStatus],
    pub index: &'a BTreeMap<String, IndexEntry>,
    pub plan: &'a Plan,
    pub limits: GuardLimits,
    /// RN-36: nenhuma execução bem-sucedida até agora.
    pub first_run: bool,
}

/// Entrada da guarda de uma tarefa que envia compactado. Ela só manda para a lixeira
/// versões que ela mesma criou, então não há limite de exclusão em massa (RN-42).
pub struct ArchiveGuardInput<'a> {
    pub sources: &'a [SourceStatus],
    /// Quantas entradas (arquivos e pastas) iriam para o arquivo compactado.
    pub entries: usize,
    /// Já existe versão enviada para este destino.
    pub has_versions: bool,
    /// RN-59: nada mudou desde a última versão — a execução não envia nada.
    pub unchanged: bool,
    /// RN-36: nenhuma execução bem-sucedida até agora.
    pub first_run: bool,
}

pub fn evaluate(input: &GuardInput<'_>) -> GuardVerdict {
    let mut abort = unavailable_sources(input.sources);
    for s in input.sources {
        if s.exists && s.readable && s.files == 0 {
            // RN-41: ponto de montagem vazio parece "tudo apagado".
            let indexed = count_files(input.index, Some(&s.name));
            if indexed > 0 {
                abort.push(format!(
                    "a fonte '{}' está vazia, mas tinha {indexed} arquivo(s) sincronizado(s) — disco desmontado?",
                    s.path.display()
                ));
            }
        }
    }
    if !abort.is_empty() {
        return GuardVerdict::Abort(abort);
    }

    let mut confirm = Vec::new();
    if input.first_run && input.plan.changes_remote() {
        confirm.push("primeira execução desta tarefa — revise o plano".to_string());
    }
    // RN-42
    let trash = input.plan.stats.trash_files;
    let indexed = count_files(input.index, None);
    let limits = input.limits;
    let over_count = trash > limits.max_trash_files;
    let over_percent = indexed > 0
        && u64::from(trash) * 100 > u64::from(limits.max_trash_percent) * indexed as u64;
    if over_count || over_percent {
        confirm.push(format!(
            "{trash} arquivo(s) iriam para a lixeira (limites: {} arquivos ou {}% de {indexed})",
            limits.max_trash_files, limits.max_trash_percent
        ));
    }

    if confirm.is_empty() {
        GuardVerdict::Proceed
    } else {
        GuardVerdict::NeedsConfirmation(confirm)
    }
}

/// Guarda de uma tarefa que envia compactado (RN-36, RN-40, RN-41, RN-43).
pub fn evaluate_archive(input: &ArchiveGuardInput<'_>) -> GuardVerdict {
    let mut abort = unavailable_sources(input.sources);
    for s in input.sources {
        // RN-41: com o disco desmontado, a versão nova sairia vazia.
        if s.exists && s.readable && s.files == 0 && input.has_versions {
            abort.push(format!(
                "a fonte '{}' está vazia, mas as versões anteriores não — disco desmontado?",
                s.path.display()
            ));
        }
    }
    if abort.is_empty() && input.entries == 0 {
        abort.push(
            "nada para compactar (tudo foi excluído pelos filtros ou não pôde ser lido)".into(),
        );
    }
    if !abort.is_empty() {
        return GuardVerdict::Abort(abort);
    }
    if input.first_run && !input.unchanged {
        return GuardVerdict::NeedsConfirmation(vec![
            "primeira execução desta tarefa — revise o que será enviado".into(),
        ]);
    }
    GuardVerdict::Proceed
}

/// RN-40: fontes que sumiram ou não podem ser lidas bloqueiam a execução.
fn unavailable_sources(sources: &[SourceStatus]) -> Vec<String> {
    sources
        .iter()
        .filter_map(|s| {
            if !s.exists {
                Some(format!("a fonte '{}' não foi encontrada", s.path.display()))
            } else if !s.readable {
                Some(format!("a fonte '{}' não pôde ser lida", s.path.display()))
            } else {
                None
            }
        })
        .collect()
}

fn count_files(index: &BTreeMap<String, IndexEntry>, under: Option<&str>) -> usize {
    index
        .values()
        .filter(|e| e.kind == EntryKind::File)
        .filter(|e| under.is_none_or(|root| is_within_str(&e.path, root)))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Action, PlanStats};
    use chrono::Utc;

    fn source(name: &str, exists: bool, files: usize) -> SourceStatus {
        SourceStatus {
            name: name.into(),
            path: format!("/home/u/{name}").into(),
            exists,
            readable: exists,
            files,
        }
    }

    fn index(paths: &[&str]) -> BTreeMap<String, IndexEntry> {
        paths
            .iter()
            .map(|p| {
                (
                    p.to_string(),
                    IndexEntry {
                        path: p.to_string(),
                        kind: EntryKind::File,
                        local_size: 1,
                        local_mtime_ns: 1,
                        local_hash: None,
                        remote_id: p.to_string(),
                        remote_hash: None,
                        synced_at: Utc::now(),
                    },
                )
            })
            .collect()
    }

    fn plan_trashing(n: u32) -> Plan {
        Plan {
            actions: vec![Action::ForgetIndex { path: "x".into() }],
            stats: PlanStats {
                trash_files: n,
                ..PlanStats::default()
            },
        }
    }

    fn eval(
        sources: &[SourceStatus],
        idx: &BTreeMap<String, IndexEntry>,
        plan: &Plan,
    ) -> GuardVerdict {
        evaluate(&GuardInput {
            sources,
            index: idx,
            plan,
            limits: GuardLimits::default(),
            first_run: false,
        })
    }

    #[test]
    fn rn40_fonte_ausente_aborta() {
        let v = eval(&[source("Docs", false, 0)], &index(&[]), &Plan::default());
        assert!(matches!(v, GuardVerdict::Abort(_)));
    }

    #[test]
    fn rn41_fonte_vazia_com_indice_aborta() {
        let idx = index(&["Docs/a", "Fotos/b"]);
        let v = eval(
            &[source("Docs", true, 0), source("Fotos", true, 1)],
            &idx,
            &Plan::default(),
        );
        assert!(matches!(v, GuardVerdict::Abort(r) if r.len() == 1));
        // Fonte nova e vazia é normal.
        let v = eval(&[source("Novo", true, 0)], &idx, &Plan::default());
        assert_eq!(v, GuardVerdict::Proceed);
    }

    #[test]
    fn rn42_exclusao_em_massa_pede_confirmacao() {
        let paths: Vec<String> = (0..200).map(|i| format!("Docs/{i}")).collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let idx = index(&refs);
        let src = [source("Docs", true, 150)];
        assert_eq!(eval(&src, &idx, &plan_trashing(10)), GuardVerdict::Proceed);
        // > 50 arquivos
        assert!(matches!(
            eval(&src, &idx, &plan_trashing(51)),
            GuardVerdict::NeedsConfirmation(_)
        ));
        // > 25 %
        let small = index(&["Docs/a", "Docs/b", "Docs/c"]);
        assert!(matches!(
            eval(&[source("Docs", true, 2)], &small, &plan_trashing(1)),
            GuardVerdict::NeedsConfirmation(_)
        ));
    }

    #[test]
    fn rn36_primeira_execucao_pede_confirmacao() {
        let plan = Plan {
            actions: vec![Action::Upload {
                path: "Docs/a".into(),
                size: 1,
            }],
            stats: PlanStats {
                uploads: 1,
                ..PlanStats::default()
            },
        };
        let v = evaluate(&GuardInput {
            sources: &[source("Docs", true, 1)],
            index: &index(&[]),
            plan: &plan,
            limits: GuardLimits::default(),
            first_run: true,
        });
        assert!(matches!(v, GuardVerdict::NeedsConfirmation(_)));
    }

    fn archive(sources: &[SourceStatus], has_versions: bool, first_run: bool) -> GuardVerdict {
        evaluate_archive(&ArchiveGuardInput {
            sources,
            entries: sources.iter().map(|s| s.files).sum(),
            has_versions,
            unchanged: false,
            first_run,
        })
    }

    #[test]
    fn rn40_rn41_compactado_bloqueia_fonte_ausente_ou_esvaziada() {
        assert!(matches!(
            archive(&[source("Docs", false, 0)], true, false),
            GuardVerdict::Abort(_)
        ));
        // Vazia com versões anteriores: provável disco desmontado.
        assert!(matches!(
            archive(&[source("Docs", true, 0), source("Fotos", true, 2)], true, false),
            GuardVerdict::Abort(r) if r.len() == 1
        ));
        // Sem nada para compactar.
        assert!(matches!(
            archive(&[source("Docs", true, 0)], false, false),
            GuardVerdict::Abort(_)
        ));
        assert_eq!(
            archive(&[source("Docs", true, 3)], true, false),
            GuardVerdict::Proceed
        );
    }

    #[test]
    fn rn36_compactado_pede_confirmacao_na_primeira_vez() {
        assert!(matches!(
            archive(&[source("Docs", true, 3)], false, true),
            GuardVerdict::NeedsConfirmation(_)
        ));
        // Nada mudou: não há o que confirmar.
        let v = evaluate_archive(&ArchiveGuardInput {
            sources: &[source("Docs", true, 3)],
            entries: 3,
            has_versions: true,
            unchanged: true,
            first_run: true,
        });
        assert_eq!(v, GuardVerdict::Proceed);
    }
}
