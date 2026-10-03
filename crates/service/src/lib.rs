//! Casos de uso do Syncly. GUI (Tauri) e CLI são adaptadores finos sobre este crate,
//! e só trocam DTOs com ele (ver `dto`).

pub mod dto;
mod error;
pub mod logging;
mod service;

pub use error::{ServiceError, ServiceResult};
pub use service::Service;
pub use syncly_core::paths::AppPaths;
