//! Provedores sem dependências externas. Provedores de nuvem ficam em crates próprios.

pub mod local;

pub use local::LocalProvider;
