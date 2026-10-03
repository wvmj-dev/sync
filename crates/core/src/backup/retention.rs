//! Retenção de backups estilo GFS (RN-54, RN-55).
//! "1 por dia nos 7 dias mais recentes que tiverem backup", idem semana e mês.

use std::collections::{BTreeSet, HashSet};

use chrono::{Datelike, NaiveDateTime};

use crate::domain::Retention;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionPlan<T> {
    pub keep: Vec<T>,
    pub remove: Vec<T>,
}

/// `backups`: (id, data/hora local). Função pura — quem chama aplica a remoção
/// (para a lixeira) só depois de um backup novo verificado (RN-55).
pub fn plan_retention<T: Clone + Ord>(
    backups: &[(T, NaiveDateTime)],
    policy: &Retention,
) -> RetentionPlan<T> {
    let mut sorted: Vec<_> = backups.iter().collect();
    sorted.sort_by_key(|b| std::cmp::Reverse(b.1));

    let mut keep = BTreeSet::new();
    // Nunca remove o mais recente (RN-55), mesmo com política zerada.
    for (id, _) in sorted.iter().take(usize::from(policy.keep_last.max(1))) {
        keep.insert(id.clone());
    }
    keep_one_per_bucket(&sorted, policy.daily, &mut keep, |t| {
        (t.year(), t.ordinal())
    });
    keep_one_per_bucket(&sorted, policy.weekly, &mut keep, |t| {
        let w = t.iso_week();
        (w.year(), w.week())
    });
    keep_one_per_bucket(&sorted, policy.monthly, &mut keep, |t| {
        (t.year(), t.month())
    });

    let mut plan = RetentionPlan {
        keep: Vec::new(),
        remove: Vec::new(),
    };
    for (id, _) in sorted {
        if keep.contains(id) {
            plan.keep.push(id.clone());
        } else {
            plan.remove.push(id.clone());
        }
    }
    plan
}

/// Percorre do mais novo para o mais velho e guarda o mais novo de cada período.
fn keep_one_per_bucket<T: Clone + Ord, K: std::hash::Hash + Eq>(
    sorted: &[&(T, NaiveDateTime)],
    count: u16,
    keep: &mut BTreeSet<T>,
    bucket: impl Fn(&NaiveDateTime) -> K,
) {
    let mut seen = HashSet::new();
    for (id, t) in sorted {
        if seen.len() >= usize::from(count) {
            break;
        }
        if seen.insert(bucket(t)) {
            keep.insert(id.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, NaiveDate};

    fn at(days_ago: i64, hour: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 3)
            .unwrap()
            .and_hms_opt(hour, 0, 0)
            .unwrap()
            - Duration::days(days_ago)
    }

    #[test]
    fn mantem_ultimos_e_um_por_dia() {
        // 3 backups por dia nos últimos 10 dias.
        let backups: Vec<(u32, NaiveDateTime)> = (0..10)
            .flat_map(|d| [6, 12, 18].map(move |h| (d * 100 + h, at(i64::from(d), h))))
            .collect();
        let policy = Retention {
            keep_last: 3,
            daily: 7,
            weekly: 0,
            monthly: 0,
        };
        let plan = plan_retention(&backups, &policy);
        // 3 de hoje (últimos) + o mais recente de cada um dos 6 dias anteriores.
        assert_eq!(plan.keep.len(), 9);
        assert_eq!(plan.keep[0], 18);
        assert!(plan.keep.contains(&118));
        assert!(!plan.keep.contains(&106));
        assert_eq!(plan.keep.len() + plan.remove.len(), backups.len());
    }

    #[test]
    fn rn55_nunca_remove_o_mais_recente() {
        let backups = [(1, at(5, 1)), (2, at(0, 1))];
        let policy = Retention {
            keep_last: 0,
            daily: 0,
            weekly: 0,
            monthly: 0,
        };
        let plan = plan_retention(&backups, &policy);
        assert_eq!(plan.keep, [2]);
        assert_eq!(plan.remove, [1]);
    }

    #[test]
    fn mensal_guarda_um_por_mes() {
        let backups: Vec<(i64, NaiveDateTime)> = (0..120).map(|d| (d, at(d, 3))).collect();
        let policy = Retention {
            keep_last: 1,
            daily: 0,
            weekly: 0,
            monthly: 4,
        };
        let plan = plan_retention(&backups, &policy);
        // Out, Set, Ago, Jul — o mais novo de cada.
        assert_eq!(plan.keep.len(), 4);
    }
}
