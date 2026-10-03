use std::fmt;
use std::path::{Path, PathBuf};

use crate::provider::ProviderError;

/// Um problema de validação, apontando o campo e o motivo (mensagem para o usuário).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationIssue {
    pub field: String,
    pub message: String,
}

impl ValidationIssue {
    pub fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ValidationIssue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

fn join_issues(issues: &[ValidationIssue]) -> String {
    issues
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("dados inválidos — {}", join_issues(.0))]
    Invalid(Vec<ValidationIssue>),

    #[error("erro de E/S em {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("configuração inválida: {0}")]
    Config(String),

    #[error("banco de estado: {0}")]
    State(#[from] rusqlite::Error),

    #[error(transparent)]
    Provider(#[from] ProviderError),

    #[error("destino: {0}")]
    Destination(String),

    #[error("esta tarefa já está em execução")]
    JobLocked,

    #[error("não foi possível localizar a pasta pessoal do usuário")]
    NoHomeDir,
}

pub type CoreResult<T> = Result<T, CoreError>;

/// Anexa o caminho a erros de E/S, para mensagens úteis.
pub(crate) trait IoContext<T> {
    fn at(self, path: &Path) -> CoreResult<T>;
}

impl<T> IoContext<T> for std::io::Result<T> {
    fn at(self, path: &Path) -> CoreResult<T> {
        self.map_err(|source| CoreError::Io {
            path: path.to_path_buf(),
            source,
        })
    }
}
