use syncly_core::CoreError;

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error("{0} não encontrada")]
    NotFound(&'static str),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Unavailable(String),
    #[error("falha interna: {0}")]
    Internal(String),
}

impl ServiceError {
    /// Código estável para a UI decidir como exibir (SEG-33: sem detalhes internos).
    pub fn code(&self) -> &'static str {
        match self {
            Self::Core(CoreError::Invalid(_)) | Self::Invalid(_) => "invalid",
            Self::Core(CoreError::JobLocked) => "busy",
            Self::Core(CoreError::Provider(_) | CoreError::Destination(_)) => "provider",
            Self::Core(_) | Self::Internal(_) => "internal",
            Self::NotFound(_) => "not_found",
            Self::Unavailable(_) => "unavailable",
        }
    }
}

pub type ServiceResult<T> = Result<T, ServiceError>;
