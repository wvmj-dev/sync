//! Comandos expostos à webview. Adaptadores finos sobre `syncly-service`:
//! convertem argumentos, chamam o caso de uso e devolvem DTOs.
//! Novo comando? Declare em `build.rs` e libere em `capabilities/default.json` (SEG-30).
//!
//! Todos são `async` para não bloquear a thread principal (a janela).

use std::path::PathBuf;

use serde::Serialize;
use syncly_service::dto::{
    AccountSummary, AppInfo, JobDraft, JobSummary, PlanPreview, RemoteItem, RunReport, VersionItem,
};
use syncly_service::{
    CompressRequest, CompressResult, ExtractResult, Progress, RunOptions, RunTrigger, Service,
    ServiceError,
};
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

/// Erro como a UI recebe: código estável + mensagem em português (SEG-33).
#[derive(Debug, Serialize)]
pub struct ApiError {
    code: &'static str,
    message: String,
}

impl From<ServiceError> for ApiError {
    fn from(err: ServiceError) -> Self {
        let code = err.code();
        if code == "internal" {
            tracing::error!("erro interno: {err}");
            return Self {
                code,
                message: "Erro interno — detalhes nos logs.".into(),
            };
        }
        Self {
            code,
            message: err.to_string(),
        }
    }
}

type ApiResult<T> = Result<T, ApiError>;

#[tauri::command]
pub async fn app_info(service: State<'_, Service>) -> ApiResult<AppInfo> {
    Ok(service.app_info())
}

#[tauri::command]
pub async fn list_accounts(service: State<'_, Service>) -> ApiResult<Vec<AccountSummary>> {
    Ok(service.list_accounts()?)
}

#[tauri::command]
pub async fn add_local_account(
    service: State<'_, Service>,
    label: String,
    root: PathBuf,
) -> ApiResult<AccountSummary> {
    Ok(service.add_local_account(label, root)?)
}

#[tauri::command]
pub async fn remove_account(service: State<'_, Service>, id: Uuid) -> ApiResult<()> {
    Ok(service.remove_account(id).await?)
}

#[tauri::command]
pub async fn list_jobs(service: State<'_, Service>) -> ApiResult<Vec<JobSummary>> {
    Ok(service.list_jobs()?)
}

#[tauri::command]
pub async fn save_job(service: State<'_, Service>, draft: JobDraft) -> ApiResult<JobSummary> {
    Ok(service.save_job(draft)?)
}

#[tauri::command]
pub async fn delete_job(service: State<'_, Service>, id: Uuid) -> ApiResult<()> {
    Ok(service.delete_job(id)?)
}

#[tauri::command]
pub async fn preview_job(service: State<'_, Service>, id: Uuid) -> ApiResult<PlanPreview> {
    Ok(service.preview_job(id).await?)
}

/// Evento `run-progress`: um por item processado.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RunProgressEvent {
    job_id: Uuid,
    #[serde(flatten)]
    progress: Progress,
}

/// Executa a sincronização. Sem `confirm`, a primeira execução (e planos com exclusões
/// em massa) volta com o código `needs_confirmation` para a UI perguntar ao usuário.
#[tauri::command]
pub async fn run_job(
    app: AppHandle,
    service: State<'_, Service>,
    id: Uuid,
    confirm: bool,
) -> ApiResult<RunReport> {
    let options = RunOptions {
        confirm,
        trigger: RunTrigger::Manual,
    };
    let report = service
        .run_job(id, options, move |p| {
            let event = RunProgressEvent {
                job_id: id,
                progress: p.clone(),
            };
            let _ = app.emit("run-progress", event);
        })
        .await?;
    Ok(report)
}

/// Versões enviadas por uma tarefa compactada (RN-50).
#[tauri::command]
pub async fn list_versions(
    service: State<'_, Service>,
    job_id: Uuid,
) -> ApiResult<Vec<VersionItem>> {
    Ok(service.list_versions(job_id)?)
}

/// Baixa, confere e descompacta uma versão numa pasta nova (RN-57, RN-58).
/// Progresso pelo evento `run-progress`, como uma execução da tarefa.
#[tauri::command]
pub async fn restore_version(
    app: AppHandle,
    service: State<'_, Service>,
    job_id: Uuid,
    version_id: Uuid,
    dest_dir: PathBuf,
) -> ApiResult<ExtractResult> {
    Ok(service
        .restore_version(job_id, version_id, &dest_dir, move |p| {
            let event = RunProgressEvent {
                job_id,
                progress: p.clone(),
            };
            let _ = app.emit("run-progress", event);
        })
        .await?)
}

/// Compacta (RN-60 a RN-64). Progresso pelo evento `archive-progress`.
#[tauri::command]
pub async fn compress_paths(
    app: AppHandle,
    service: State<'_, Service>,
    request: CompressRequest,
) -> ApiResult<CompressResult> {
    Ok(service
        .compress(request, move |p| {
            let _ = app.emit("archive-progress", p.clone());
        })
        .await?)
}

/// Descompacta numa pasta nova (RN-65 a RN-67). Progresso pelo evento `archive-progress`.
#[tauri::command]
pub async fn extract_archive(
    app: AppHandle,
    service: State<'_, Service>,
    archive: PathBuf,
    dest_dir: PathBuf,
) -> ApiResult<ExtractResult> {
    Ok(service
        .extract(archive, dest_dir, move |p| {
            let _ = app.emit("archive-progress", p.clone());
        })
        .await?)
}

#[tauri::command]
pub async fn cancel_archive(service: State<'_, Service>) -> ApiResult<bool> {
    Ok(service.cancel_archive())
}

/// RN-95: pede para parar uma execução em andamento.
#[tauri::command]
pub async fn cancel_run(service: State<'_, Service>, id: Uuid) -> ApiResult<bool> {
    Ok(service.cancel_run(id))
}

/// Abre o navegador do sistema e espera a autorização (até 5 minutos).
#[tauri::command]
pub async fn connect_google(service: State<'_, Service>) -> ApiResult<AccountSummary> {
    Ok(service.connect_google(|_| {}).await?)
}

#[tauri::command]
pub async fn list_remote(
    service: State<'_, Service>,
    account_id: Uuid,
    path: String,
) -> ApiResult<Vec<RemoteItem>> {
    Ok(service.list_remote(account_id, &path).await?)
}

#[tauri::command]
pub async fn upload_file(
    service: State<'_, Service>,
    account_id: Uuid,
    local_path: PathBuf,
    remote_dir: String,
) -> ApiResult<RemoteItem> {
    Ok(service
        .upload_file(account_id, &local_path, &remote_dir)
        .await?)
}

/// Retorna o caminho onde o arquivo foi salvo.
#[tauri::command]
pub async fn download_file(
    service: State<'_, Service>,
    account_id: Uuid,
    remote_path: String,
    local_dir: PathBuf,
) -> ApiResult<String> {
    let saved = service
        .download_file(account_id, &remote_path, &local_dir)
        .await?;
    Ok(saved.display().to_string())
}
