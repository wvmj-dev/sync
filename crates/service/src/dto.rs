//! Objetos trocados com a UI e a CLI. Espelhados em `ui/src/api/types.ts` —
//! ao mudar um campo aqui, mude lá no mesmo commit. Nada de tokens ou chaves (SEG-33).

use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use syncly_core::archive::ArchiveFormat;
use syncly_core::domain::{Action, Delivery, PlanStats, RunRecord, RunStatus, Schedule, SyncMode};
use syncly_core::sync::executor::ExecutionReport;
use syncly_core::sync::guard::GuardVerdict;
use syncly_core::sync::status::{ItemStatus, SituationCounts};
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
    pub last_run: Option<RunSummary>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSummary {
    pub account_id: Uuid,
    pub account_label: Option<String>,
    pub path: String,
    pub delivery: Delivery,
    pub mode: SyncMode,
    /// Rótulo legível ("Diário às 02:00").
    pub schedule: String,
    /// O agendamento em si, para o formulário.
    pub schedule_spec: Schedule,
    /// `manual`, `disabled`, `waiting_first_run` (RN-36), `active` ou `not_installed`.
    pub schedule_state: &'static str,
    /// RN-100: próximo disparo do timer, se ativo.
    pub next_run: Option<DateTime<Utc>>,
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
    /// RN-100: itens transferidos (enviados, atualizados, conflitos, lixeira ou extraídos).
    pub items: Option<u64>,
    /// RN-100: bytes enviados ou extraídos.
    pub bytes: Option<u64>,
}

impl From<RunRecord> for RunSummary {
    fn from(r: RunRecord) -> Self {
        let stat = |key: &str| r.stats.as_ref().and_then(|s| s.get(key)?.as_u64());
        let items: Vec<u64> = ["uploaded", "updated", "conflicts", "trashed", "files"]
            .iter()
            .filter_map(|k| stat(k))
            .collect();
        Self {
            kind: r.kind.as_str(),
            trigger: r.trigger.as_str(),
            status: r.status.as_str(),
            started_at: r.started_at,
            finished_at: r.finished_at,
            error: r.error,
            items: (!items.is_empty()).then(|| items.iter().sum()),
            bytes: stat("bytesSent").or_else(|| stat("bytes")),
        }
    }
}

/// Status de uma pasta de uma tarefa (RN-100 a RN-103).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    pub job_id: Uuid,
    /// Pasta mostrada, relativa ao destino (`""` = raiz da tarefa).
    pub folder: String,
    /// Só tarefas que enviam arquivo por arquivo têm status por arquivo; as compactadas
    /// mostram as versões.
    pub per_file: bool,
    /// Itens dentro de `folder` (até `MAX_STATUS_ITEMS`; total em `total_items`).
    pub items: Vec<ItemStatus>,
    pub total_items: usize,
    /// Arquivos da tarefa inteira em cada situação.
    pub counts: SituationCounts,
    /// RN-100: no computador agora.
    pub local_files: u64,
    pub local_bytes: u64,
    /// RN-100: no último sync.
    pub synced_files: u64,
    pub synced_bytes: u64,
    /// RN-103: quando o destino foi consultado; `None` = dados do último sync.
    pub remote_checked_at: Option<DateTime<Utc>>,
    pub duration_ms: u64,
}

pub const MAX_STATUS_ITEMS: usize = 1000;

/// Criação/edição de tarefa vinda da UI. Campos que a UI ainda não edita
/// (agendamento, guardas) são preservados ao editar.
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
    /// RN-50: arquivo por arquivo (padrão) ou compactado com versões.
    #[serde(default)]
    pub delivery: Delivery,
    /// Só vale para a entrega arquivo por arquivo.
    #[serde(default)]
    pub mode: SyncMode,
    /// RN-80. `None` mantém o agendamento atual.
    #[serde(default)]
    pub schedule: Option<Schedule>,
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
    /// Presente quando a tarefa envia compactado; aí `stats` e `actions` ficam vazios.
    pub archive: Option<ArchivePreview>,
    /// Presente quando a tarefa recebe; aí `stats` e `actions` ficam vazios.
    pub receive: Option<ReceivePreview>,
}

/// O que uma execução de recebimento faria agora (RN-107).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceivePreview {
    /// Versão mais recente no destino; `None` se não houver nenhuma.
    pub name: Option<String>,
    pub size: Option<u64>,
    pub modified_at: Option<DateTime<Utc>>,
    /// Já foi recebida — nada será feito.
    pub already_received: bool,
    /// Pasta do computador onde a pasta nova será criada.
    pub target: String,
}

/// Resultado de uma execução de recebimento.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiveRun {
    pub name: String,
    /// Pasta criada (ou, se nada mudou, a do último recebimento).
    pub dest: String,
    pub files: u64,
    pub bytes: u64,
    /// RN-66: entradas ignoradas por segurança.
    pub skipped_count: usize,
    /// A versão mais recente já tinha sido recebida.
    pub unchanged: bool,
}

/// O que uma execução compactada faria agora (RN-50 a RN-59).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchivePreview {
    /// Nome que a versão teria se fosse criada agora.
    pub name: String,
    pub format: ArchiveFormat,
    pub files: u64,
    pub folders: u64,
    /// Tamanho antes de compactar.
    pub bytes: u64,
    /// RN-59: nada mudou desde a última versão — nada será enviado.
    pub unchanged: bool,
    /// Versões já enviadas para este destino.
    pub versions: usize,
    /// RN-54: versões que iriam para a lixeira depois do envio.
    pub to_trash: Vec<String>,
}

/// Resultado específico de uma execução compactada.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveRun {
    /// Versão criada; `None` quando nada mudou (RN-59).
    pub name: Option<String>,
    pub files: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub unchanged: bool,
    /// Versões mantidas no destino depois da retenção.
    pub versions: usize,
}

/// Versão enviada por uma tarefa compactada (RN-50), para listar e restaurar.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionItem {
    pub id: Uuid,
    pub name: String,
    pub size: u64,
    pub created_at: DateTime<Utc>,
    pub account_id: Uuid,
    /// `None` se a conta foi removida.
    pub account_label: Option<String>,
    /// Pasta no destino, relativa à raiz do app.
    pub path: String,
}

/// "Enviar compactado" avulso (RN-109): compacta e envia um único arquivo.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadCompressedRequest {
    pub account_id: Uuid,
    /// Pastas e/ou arquivos locais (1 a 32).
    pub inputs: Vec<PathBuf>,
    /// Pasta dentro de `Syncly/` (criada se faltar); `""` = a raiz.
    #[serde(default)]
    pub remote_dir: String,
    pub format: ArchiveFormat,
    /// Nome sem extensão; padrão: o da única entrada, ou "arquivos".
    #[serde(default)]
    pub name: Option<String>,
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
    /// Presente quando a tarefa envia compactado.
    pub archive: Option<ArchiveRun>,
    /// Presente quando a tarefa recebe.
    pub receive: Option<ReceiveRun>,
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
            archive: None,
            receive: None,
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
