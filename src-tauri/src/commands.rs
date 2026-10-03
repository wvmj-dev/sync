//! Comandos expostos à webview. Adaptadores finos sobre `syncly-service`:
//! convertem argumentos, chamam o caso de uso e devolvem DTOs.
//! Novo comando? Declare em `build.rs` e libere em `capabilities/default.json` (SEG-30).
//!
//! Todos são `async` para não bloquear a thread principal (a janela).

use std::path::PathBuf;

use serde::Serialize;
use syncly_service::dto::{AccountSummary, AppInfo, JobDraft, JobSummary, PlanPreview};
use syncly_service::{Service, ServiceError};
use tauri::State;
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
    Ok(service.remove_account(id)?)
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
