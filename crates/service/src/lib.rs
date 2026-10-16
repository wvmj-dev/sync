//! Casos de uso do Syncly. GUI (Tauri) e CLI são adaptadores finos sobre este crate,
//! e só trocam DTOs com ele (ver `dto`).

mod archive;
mod auth_watch;
mod delivery;
pub mod dto;
mod encryption;
mod error;
mod google;
pub mod logging;
mod os;
mod receive;
mod restore;
mod schedule;
pub mod secrets;
mod service;
mod status;
#[cfg(test)]
mod test_support;
mod transfers;

pub use archive::{CompressRequest, CompressResult, ExtractResult};
pub use error::{ServiceError, ServiceResult};
pub use google::GoogleBackend;
pub use os::{LinuxDesktop, OsIntegration};
pub use service::{RunOptions, Service};
pub use syncly_core::archive::{ArchiveFormat, ArchiveProgress};
pub use syncly_core::domain::RunTrigger;
pub use syncly_core::paths::AppPaths;
pub use syncly_core::sync::executor::Progress;
pub use syncly_core::timers::EXIT_RETRY;
