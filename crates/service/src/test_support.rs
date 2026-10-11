//! Apoio aos testes: cofre em memória e Google simulado — nunca o keyring nem a conta
//! reais (AGENTS.md, regra 9).

use std::path::Path;
use std::sync::Arc;

use syncly_core::paths::AppPaths;
use syncly_gdrive::Endpoints;

use crate::google::GoogleBackend;
use crate::secrets::MemoryStore;
use crate::service::Service;

pub const GOOGLE_CLIENT_JSON: &str =
    r#"{"installed":{"client_id":"1-a.apps.googleusercontent.com","client_secret":"s"}}"#;

/// Serviço isolado em `base`, com o Google apontando para `google_base`.
pub fn service(base: &Path, google_base: &str) -> Service {
    Service::with_backends(
        AppPaths::in_dir(base),
        Arc::new(MemoryStore::default()),
        GoogleBackend {
            endpoints: Endpoints::under(google_base),
            open_browser: false,
        },
    )
    .expect("serviço de teste")
}
