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
    /// RN-36, RN-42: o plano precisa ser revisado e confirmado.
    #[error("exige confirmação — {}", .0.join("; "))]
    NeedsConfirmation(Vec<String>),
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
            Self::NeedsConfirmation(_) => "needs_confirmation",
        }
    }
}

pub type ServiceResult<T> = Result<T, ServiceError>;
