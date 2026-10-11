//! Objetos trocados com a UI e a CLI. Espelhados em `ui/src/api/types.ts` —
//! ao mudar um campo aqui, mude lá no mesmo commit. Nada de tokens ou chaves (SEG-33).

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use syncly_core::domain::{Action, PlanStats, RunRecord, RunStatus, SyncMode};
use syncly_core::sync::executor::ExecutionReport;
use syncly_core::sync::guard::GuardVerdict;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: &'static str,
    pub config_file: String,
    pub data_dir: String,
    pub log_dir: String,
    pub google_client_configured: bool,
    /// Problema no `google-client.json` (ausente não conta como problema).
    pub google_client_problem: Option<String>,
    /// Problema ao ler o `config.toml`.
    pub config_problem: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummary {
    pub id: Uuid,
    pub label: String,
    /// `"google_drive"` ou `"local"`.
    pub provider: &'static str,
    /// E-mail (Drive) ou pasta raiz (local).
    pub detail: String,
    pub job_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSummary {
    pub id: Uuid,
    pub name: String,
    pub enabled: bool,
    pub sources: Vec<String>,
    pub exclude: Vec<String>,
    pub include_sensitive: bool,
    pub sync: Option<SyncSummary>,
    pub backup_enabled: bool,
    pub last_run: Option<RunSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSummary {
    pub account_id: Uuid,
    pub account_label: Option<String>,
    pub path: String,
    pub mode: SyncMode,
    pub schedule: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub kind: &'static str,
    pub trigger: &'static str,
    pub status: &'static str,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

impl From<RunRecord> for RunSummary {
    fn from(r: RunRecord) -> Self {
        Self {
            kind: r.kind.as_str(),
            trigger: r.trigger.as_str(),
            status: r.status.as_str(),
            started_at: r.started_at,
            finished_at: r.finished_at,
            error: r.error,
        }
    }
}

/// Criação/edição de tarefa vinda da UI. Campos que a UI ainda não edita
/// (agendamento, guardas, backup) são preservados ao editar.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobDraft {
    pub id: Option<Uuid>,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub sources: Vec<PathBuf>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub include_sensitive: bool,
    pub sync: Option<SyncDraft>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncDraft {
    pub account_id: Uuid,
    pub path: String,
    #[serde(default)]
    pub mode: SyncMode,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanPreview {
    pub job_id: Uuid,
    pub mode: SyncMode,
    pub stats: PlanStats,
    pub verdict: GuardVerdict,
    /// Primeiras `MAX_PREVIEW_ACTIONS` ações.
    pub actions: Vec<ActionView>,
    pub total_actions: usize,
    pub warnings: Vec<String>,
    pub hashed_files: usize,
    pub duration_ms: u64,
    pub first_run: bool,
}

/// Item listado numa conta (arquivo ou pasta dentro de `Syncly/`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteItem {
    pub name: String,
    /// Relativo à pasta raiz do app.
    pub path: String,
    /// `"file"` ou `"folder"`.
    pub kind: &'static str,
    pub size: Option<u64>,
    pub modified_at: Option<DateTime<Utc>>,
    /// RN-101: autor da última modificação, quando o provedor informa.
    pub modified_by: Option<String>,
    /// Arquivos sem checksum (ex.: Google Docs nativos) não são baixados.
    pub downloadable: bool,
}

/// Resultado de uma execução. Falhas limitadas a `MAX_REPORT_FAILURES` para a UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReport {
    /// RN-91: `success`, `partial_success`, `failed`, `aborted`, `cancelled`…
    pub status: &'static str,
    #[serde(flatten)]
    pub report: ExecutionReport,
    pub failure_count: usize,
    pub duration_ms: u64,
}

pub const MAX_REPORT_FAILURES: usize = 50;

impl RunReport {
    pub fn new(status: RunStatus, mut report: ExecutionReport, duration_ms: u64) -> Self {
        let failure_count = report.failures.len();
        report.failures.truncate(MAX_REPORT_FAILURES);
        Self {
            status: status.as_str(),
            report,
            failure_count,
            duration_ms,
        }
    }
}

pub const MAX_PREVIEW_ACTIONS: usize = 500;
pub const MAX_PREVIEW_WARNINGS: usize = 100;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionView {
    /// `create_folder`, `upload`, `update`, `conflict`, `adopt`, `trash`, `forget`, `skip`.
    pub kind: &'static str,
    pub path: String,
    pub detail: Option<String>,
}

impl From<&Action> for ActionView {
    fn from(a: &Action) -> Self {
        let (kind, detail) = match a {
            Action::CreateFolder { .. } => ("create_folder", None),
            Action::Upload { size, .. } => ("upload", Some(human_bytes(*size))),
            Action::Update { size, .. } => ("update", Some(human_bytes(*size))),
            Action::ResolveConflict { conflict_name, .. } => (
                "conflict",
                Some(format!(
                    "versão remota será renomeada para \"{conflict_name}\""
                )),
            ),
            Action::Adopt { .. } => ("adopt", Some("já idêntico no destino".into())),
            Action::Trash { .. } => ("trash", Some("vai para a lixeira do destino".into())),
            Action::ForgetIndex { .. } => ("forget", None),
            Action::Skip { reason, .. } => ("skip", Some(reason.label().into())),
        };
        Self {
            kind,
            path: a.path().to_string(),
            detail,
        }
    }
}

pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_legiveis() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(5 * 1024 * 1024 * 1024), "5.0 GiB");
    }
}
