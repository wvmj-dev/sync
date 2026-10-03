use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::check_label;
use crate::error::ValidationIssue;

/// Um destino de armazenamento (RN-01 a RN-07). Tokens NÃO ficam aqui — ficam no keyring (SEG-04).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: Uuid,
    pub label: String,
    #[serde(flatten)]
    pub kind: AccountKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum AccountKind {
    GoogleDrive {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        email: Option<String>,
        /// ID da pasta `Syncly/` (RN-02). Preenchido ao conectar.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        root_folder_id: Option<String>,
    },
    /// Pasta local tratada como raiz do app (RN-07).
    Local { root: PathBuf },
}

impl AccountKind {
    pub fn provider_id(&self) -> &'static str {
        match self {
            Self::GoogleDrive { .. } => "google_drive",
            Self::Local { .. } => "local",
        }
    }
}

impl Account {
    pub fn validate(&self) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        let field = |f: &str| format!("conta '{}'.{f}", self.label);
        if let Err(e) = check_label(&self.label, 64) {
            issues.push(ValidationIssue::new(field("nome"), e));
        }
        if let AccountKind::Local { root } = &self.kind
            && !root.is_absolute()
        {
            issues.push(ValidationIssue::new(
                field("pasta"),
                "deve ser um caminho absoluto",
            ));
        }
        issues
    }
}
