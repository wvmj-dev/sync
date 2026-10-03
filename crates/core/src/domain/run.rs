use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

macro_rules! str_enum {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),+ }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }

            pub fn parse(value: &str) -> Option<Self> {
                match value { $($text => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

str_enum!(RunKind {
    Sync => "sync",
    Backup => "backup",
    Restore => "restore",
    Compress => "compress",
});

str_enum!(RunTrigger {
    Manual => "manual",
    Scheduled => "scheduled",
});

// RN-91
str_enum!(RunStatus {
    Running => "running",
    Success => "success",
    PartialSuccess => "partial_success",
    Failed => "failed",
    Aborted => "aborted",
    AwaitingConfirmation => "awaiting_confirmation",
    Cancelled => "cancelled",
    Postponed => "postponed",
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunRecord {
    pub id: Uuid,
    pub job_id: Uuid,
    pub kind: RunKind,
    pub trigger: RunTrigger,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub stats: Option<serde_json::Value>,
    pub error: Option<String>,
}
