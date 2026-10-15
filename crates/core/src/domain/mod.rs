//! Tipos de domínio e suas validações. Sem I/O.

pub mod account;
pub mod archive_format;
pub mod entry;
pub mod job;
pub mod plan;
pub mod rel_path;
pub mod run;
pub mod schedule;

pub use account::{Account, AccountKind};
pub use archive_format::ArchiveFormat;
pub use entry::{EntryKind, IndexEntry, LocalEntry};
pub use job::{
    Delivery, Destination, GuardLimits, Job, Retention, SyncMode, SyncSettings, ValidationContext,
};
pub use plan::{Action, Plan, PlanStats, SkipReason};
pub use rel_path::RelPath;
pub use run::{RunKind, RunRecord, RunStatus, RunTrigger};
pub use schedule::{Schedule, TimeOfDay, Weekday};

/// Nome de exibição: 1 a `max` caracteres, sem caracteres de controle (RN-10).
pub(crate) fn check_label(value: &str, max: usize) -> Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("é obrigatório".into());
    }
    if trimmed.chars().count() > max {
        return Err(format!("deve ter no máximo {max} caracteres"));
    }
    if value.chars().any(char::is_control) {
        return Err("não pode conter caracteres de controle".into());
    }
    Ok(())
}
