//! Interface que todo provedor de armazenamento implementa (Drive, pasta local,
//! futuros OneDrive/Dropbox/S3). IDs são opacos para o núcleo.

use std::path::Path;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::EntryKind;
pub use crate::hash::HashKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteEntry {
    pub id: String,
    pub name: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
    /// Hash no algoritmo de [`StorageProvider::hash_kind`]. `None` = sem checksum (RN-34).
    pub hash: Option<String>,
    pub modified_at: Option<DateTime<Utc>>,
    /// Autor da última modificação, quando o provedor informa (RN-101).
    pub modified_by: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageQuota {
    /// `None` = ilimitado.
    pub limit: Option<u64>,
    pub used: u64,
}

impl StorageQuota {
    pub fn free(&self) -> Option<u64> {
        self.limit.map(|l| l.saturating_sub(self.used))
    }
}

/// Erros já classificados pelo provedor, para o motor decidir o que fazer (RN-96 a RN-98).
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("falha de rede: {0}")]
    Network(String),
    #[error("limite de requisições atingido")]
    RateLimited { retry_after: Option<Duration> },
    #[error("erro temporário do servidor (HTTP {status})")]
    Server {
        status: u16,
        retry_after: Option<Duration>,
    },
    #[error("checksum divergente após a transferência")]
    ChecksumMismatch,
    #[error("autorização expirada ou revogada — reconecte a conta")]
    AuthExpired,
    #[error("armazenamento da conta cheio")]
    StorageFull,
    #[error("limite diário da conta atingido")]
    DailyLimit,
    #[error("sem permissão: {0}")]
    PermissionDenied(String),
    #[error("não encontrado: {0}")]
    NotFound(String),
    #[error("já existe: {0}")]
    AlreadyExists(String),
    #[error("requisição inválida: {0}")]
    BadRequest(String),
    #[error("operação não suportada por este provedor")]
    Unsupported,
    #[error("erro de E/S local: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

/// Alcance de um erro: define se tenta de novo, se falha só o item ou se aborta tudo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorScope {
    /// RN-96: tentar de novo com backoff.
    Transient,
    /// RN-97: falha o item, os demais seguem.
    Item,
    /// RN-98: aborta a execução (e marca a conta, se for autenticação).
    Account,
}

impl ProviderError {
    pub fn scope(&self) -> ErrorScope {
        match self {
            Self::Network(_)
            | Self::RateLimited { .. }
            | Self::Server { .. }
            | Self::ChecksumMismatch => ErrorScope::Transient,
            Self::AuthExpired | Self::StorageFull | Self::DailyLimit => ErrorScope::Account,
            Self::PermissionDenied(_)
            | Self::NotFound(_)
            | Self::AlreadyExists(_)
            | Self::BadRequest(_)
            | Self::Unsupported
            | Self::Io(_)
            | Self::Other(_) => ErrorScope::Item,
        }
    }

    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RateLimited { retry_after } | Self::Server { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

pub type ProviderResult<T> = Result<T, ProviderError>;

/// Contrato de um provedor. Repare que **não existe exclusão definitiva** (SEG-10):
/// a única forma de remover algo é [`StorageProvider::trash`].
#[async_trait]
pub trait StorageProvider: Send + Sync {
    /// Identificador estável (`"google_drive"`, `"local"`…).
    fn id(&self) -> &'static str;

    /// Algoritmo de hash que o servidor informa — o scanner calcula o mesmo localmente.
    fn hash_kind(&self) -> HashKind;

    /// Pasta raiz do app nesta conta (RN-02, RN-07).
    async fn app_root(&self) -> ProviderResult<RemoteEntry>;

    async fn list_children(&self, folder_id: &str) -> ProviderResult<Vec<RemoteEntry>>;

    async fn get(&self, id: &str) -> ProviderResult<Option<RemoteEntry>>;

    async fn create_folder(&self, parent_id: &str, name: &str) -> ProviderResult<RemoteEntry>;

    /// Envia um arquivo novo. Retorna a entrada com o hash calculado pelo servidor (RN-37).
    async fn upload_new(
        &self,
        parent_id: &str,
        name: &str,
        local: &Path,
    ) -> ProviderResult<RemoteEntry>;

    /// Substitui o conteúdo de um arquivo existente (o Drive guarda a versão anterior).
    async fn upload_replace(&self, id: &str, local: &Path) -> ProviderResult<RemoteEntry>;

    async fn download(&self, id: &str, dest: &Path) -> ProviderResult<()>;

    async fn rename(&self, id: &str, new_name: &str) -> ProviderResult<RemoteEntry>;

    /// Move para a lixeira — recuperável (RN-44).
    async fn trash(&self, id: &str) -> ProviderResult<()>;

    /// Cota de armazenamento, se o provedor informar (RN-46).
    async fn quota(&self) -> ProviderResult<Option<StorageQuota>>;
}
