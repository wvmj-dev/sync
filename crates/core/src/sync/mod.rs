//! Motor de sincronização: escanear → estado remoto → planejar → guardas → executar.
//! Ver ARQUITETURA.md §6.

pub mod guard;
pub mod planner;
pub mod remote_tree;
pub mod retry;
pub mod scanner;
