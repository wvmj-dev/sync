use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    File,
    Folder,
}

impl EntryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Folder => "folder",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "file" => Some(Self::File),
            "folder" => Some(Self::Folder),
            _ => None,
        }
    }
}

/// Item encontrado nas fontes locais. A chave do mapa é o caminho relativo ao
/// destino, começando pelo nome da fonte (RN-14).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalEntry {
    pub kind: EntryKind,
    pub size: u64,
    pub mtime_ns: i64,
    /// Hash no algoritmo do provedor. Só para arquivos.
    pub hash: Option<String>,
}

/// Último estado sincronizado de um item (RN-38). Permite saber o que mudou
/// de cada lado desde a última execução.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub path: String,
    pub kind: EntryKind,
    pub local_size: u64,
    pub local_mtime_ns: i64,
    pub local_hash: Option<String>,
    pub remote_id: String,
    pub remote_hash: Option<String>,
    pub synced_at: DateTime<Utc>,
}
