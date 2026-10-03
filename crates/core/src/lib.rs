//! Núcleo do Syncly: domínio, regras de negócio e motor de sincronização.
//!
//! Este crate não conhece Tauri, a UI nem provedores específicos — eles entram
//! pela trait [`provider::StorageProvider`]. Veja `docs/ARQUITETURA.md`.
//! Regras citadas como `RN-xx`/`SEG-xx` estão em `docs/REGRAS_DE_NEGOCIO.md`
//! e `docs/SEGURANCA.md`.

pub mod backup;
pub mod config;
pub mod domain;
pub mod error;
pub mod filters;
pub mod fsutil;
pub mod hash;
pub mod lock;
pub mod paths;
pub mod provider;
pub mod providers;
pub mod state;
pub mod sync;

pub use error::{CoreError, CoreResult, ValidationIssue};
