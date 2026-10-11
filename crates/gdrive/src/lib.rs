//! Provedor Google Drive (API REST v3, escopo `drive.file`).
//!
//! - `auth`: leitura do cliente OAuth (`google-client.json`).
//! - `oauth`: login com PKCE + loopback, renovação e revogação de tokens.
//! - `client`: HTTP com limitador de taxa, retentativas e envio retomável.
//! - `provider`: implementação de `StorageProvider`.
//!
//! Ver `docs/SEGURANCA.md` (SEG-01 a SEG-08, SEG-40 a SEG-42).

pub mod auth;
pub mod client;
pub mod endpoints;
pub mod error;
pub mod oauth;
pub mod provider;
pub mod query;
mod random;

pub use client::{DriveClient, Tuning, http_client};
pub use endpoints::Endpoints;
pub use oauth::{LoginError, PendingLogin, Secret, Tokens};
pub use provider::GDriveProvider;
/// Cliente HTTP compartilhado (reuso de conexões).
pub use reqwest::Client as HttpClient;
