//! Provedor Google Drive.
//!
//! Fase 0 (esta): regras puras e testadas — configuração do cliente OAuth,
//! escape de queries e classificação de erros.
//!
//! Fase 1: cliente HTTP (`reqwest` + `rustls`), fluxo OAuth PKCE com loopback,
//! tokens no keyring e a implementação de `StorageProvider`.
//! Ver `docs/SEGURANCA.md`.

pub mod auth;
pub mod error;
pub mod query;
