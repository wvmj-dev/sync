use serde::Serialize;

use super::entry::EntryKind;

/// Ação decidida pelo planejador (RN-30). O executor só aplica o que está aqui.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    CreateFolder {
        path: String,
    },
    Upload {
        path: String,
        size: u64,
    },
    Update {
        path: String,
        remote_id: String,
        size: u64,
    },
    /// RN-35: renomeia a versão remota e envia a local com o nome original.
    ResolveConflict {
        path: String,
        remote_id: String,
        conflict_name: String,
        size: u64,
    },
    /// Conteúdo idêntico dos dois lados — só registra no índice (RN-56).
    Adopt {
        path: String,
        remote_id: String,
    },
    /// Lixeira, nunca exclusão definitiva (RN-44).
    Trash {
        path: String,
        remote_id: String,
        kind: EntryKind,
    },
    ForgetIndex {
        path: String,
    },
    /// Informativo: o item não será tocado.
    Skip {
        path: String,
        reason: SkipReason,
    },
}

impl Action {
    pub fn path(&self) -> &str {
        match self {
            Self::CreateFolder { path }
            | Self::Upload { path, .. }
            | Self::Update { path, .. }
            | Self::ResolveConflict { path, .. }
            | Self::Adopt { path, .. }
            | Self::Trash { path, .. }
            | Self::ForgetIndex { path }
            | Self::Skip { path, .. } => path,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// RN-33
    KindMismatch,
    /// RN-34
    RemoteWithoutChecksum,
    InvalidRemoteName,
    DuplicateRemoteName,
    /// Arquivo/pasta local que não pôde ser lido — nada abaixo dele é decidido.
    LocalUnreadable,
    /// Apagado localmente, mas alterado no destino depois do último sync.
    RemoteChangedAfterLocalDelete,
}

impl SkipReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::KindMismatch => "arquivo de um lado e pasta do outro",
            Self::RemoteWithoutChecksum => "item remoto sem checksum (ex.: Google Docs)",
            Self::InvalidRemoteName => "nome remoto inválido localmente",
            Self::DuplicateRemoteName => "nome duplicado no destino",
            Self::LocalUnreadable => "não foi possível ler localmente",
            Self::RemoteChangedAfterLocalDelete => {
                "apagado localmente, mas alterado no destino — mantido"
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStats {
    pub folders_to_create: u32,
    pub uploads: u32,
    pub updates: u32,
    pub conflicts: u32,
    pub adopted: u32,
    pub trash_files: u32,
    pub trash_folders: u32,
    pub forgotten: u32,
    pub skipped: u32,
    pub bytes_to_send: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Plan {
    pub actions: Vec<Action>,
    pub stats: PlanStats,
}

impl Plan {
    /// Há algo que altere o destino?
    pub fn changes_remote(&self) -> bool {
        let s = &self.stats;
        s.folders_to_create + s.uploads + s.updates + s.conflicts + s.trash_files + s.trash_folders
            > 0
    }
}
