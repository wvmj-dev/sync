use syncly_core::CoreError;
use syncly_core::provider::ProviderError;
use syncly_core::timers::{self, PostponeReason};

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
    #[error("{0}")]
    Busy(&'static str),
    /// RN-06: o Google recusou o login da conta; só reconectando.
    #[error("a conta '{0}' precisa ser reconectada: abra Contas → Reconectar")]
    NeedsReconnect(String),
    /// RN-36, RN-42: o plano precisa ser revisado e confirmado.
    #[error("exige confirmação — {}", .0.join("; "))]
    NeedsConfirmation(Vec<String>),
    /// O cofre de senhas do sistema não respondeu (bloqueado, sem Secret Service…).
    #[error("{0}")]
    Keyring(String),
    /// RN-84: execução agendada adiada; `streak` conta os adiamentos seguidos, este incluído.
    #[error("execução adiada: {}", postponed_message(*.reason, *.streak))]
    Postponed { reason: PostponeReason, streak: u32 },
}

impl ServiceError {
    /// Código estável para a UI decidir como exibir (SEG-33: sem detalhes internos).
    pub fn code(&self) -> &'static str {
        match self {
            Self::Core(CoreError::Invalid(_)) | Self::Invalid(_) => "invalid",
            Self::Core(CoreError::JobLocked) | Self::Busy(_) => "busy",
            Self::Core(CoreError::Cancelled) => "cancelled",
            Self::Core(CoreError::DiskFull(_)) => "disk_full",
            Self::Core(CoreError::Provider(_) | CoreError::Destination(_)) => "provider",
            Self::Core(_) | Self::Internal(_) => "internal",
            Self::NotFound(_) => "not_found",
            Self::Unavailable(_) | Self::Keyring(_) => "unavailable",
            Self::Postponed { .. } => "postponed",
            Self::NeedsConfirmation(_) => "needs_confirmation",
            Self::NeedsReconnect(_) => "needs_reconnect",
        }
    }

    /// RN-84: por que uma execução agendada que falhou com este erro deve ser adiada.
    pub(crate) fn postpone_reason(&self) -> Option<PostponeReason> {
        match self {
            Self::Core(CoreError::Provider(ProviderError::Network(_))) => {
                Some(PostponeReason::Offline)
            }
            Self::Keyring(_) => Some(PostponeReason::KeyringLocked),
            _ => None,
        }
    }

    /// RN-84: o processo deve sair com [`timers::EXIT_RETRY`] para o systemd tentar de novo.
    pub fn retry_later(&self) -> bool {
        matches!(self, Self::Postponed { reason, streak }
            if timers::postponement(*reason, *streak).retry)
    }
}

/// RN-84: o motivo e quando será a próxima tentativa, em uma frase.
pub(crate) fn postponed_message(reason: PostponeReason, streak: u32) -> String {
    let why = match reason {
        PostponeReason::Offline => "sem conexão com a internet",
        PostponeReason::KeyringLocked => "o cofre de senhas do sistema está bloqueado",
    };
    if timers::postponement(reason, streak).retry {
        format!("{why}; nova tentativa em {} min", timers::RETRY_MINUTES)
    } else {
        format!("{why}; nova tentativa no próximo horário agendado")
    }
}

pub type ServiceResult<T> = Result<T, ServiceError>;
