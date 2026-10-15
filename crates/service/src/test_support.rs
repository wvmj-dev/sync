//! Apoio aos testes: cofre em memória, Google simulado e um systemd de mentira — nunca o
//! keyring, a conta, o systemd nem a sessão reais (AGENTS.md, regra 9).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use syncly_core::paths::AppPaths;
use syncly_gdrive::Endpoints;

use crate::error::{ServiceError, ServiceResult};
use crate::google::GoogleBackend;
use crate::os::OsIntegration;
use crate::secrets::MemoryStore;
use crate::service::Service;

pub const GOOGLE_CLIENT_JSON: &str =
    r#"{"installed":{"client_id":"1-a.apps.googleusercontent.com","client_secret":"s"}}"#;

/// Executável que os timers de teste apontam (não precisa existir).
pub const FAKE_EXE: &str = "/opt/syncly/syncly-teste";

/// Registra o que seria feito no sistema, sem fazer nada.
#[derive(Default)]
pub struct FakeOs {
    pub calls: Mutex<Vec<String>>,
}

impl FakeOs {
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }
}

#[async_trait]
impl OsIntegration for FakeOs {
    fn exe(&self) -> ServiceResult<PathBuf> {
        Ok(PathBuf::from(FAKE_EXE))
    }

    fn systemctl(&self, args: &[&str]) -> ServiceResult<()> {
        self.record(format!("systemctl {}", args.join(" ")));
        Ok(())
    }

    /// Recusa expressões com "inválida", como o `systemd-analyze` faria.
    fn verify_calendar(&self, expr: &str) -> ServiceResult<()> {
        self.record(format!("verify {expr}"));
        if expr.contains("invalida") {
            return Err(ServiceError::Invalid(
                "expressão de agendamento inválida".into(),
            ));
        }
        Ok(())
    }

    async fn notify(&self, title: &str, body: &str) -> ServiceResult<()> {
        self.record(format!("notify {title} | {body}"));
        Ok(())
    }
}

/// Serviço isolado em `base`, com o Google apontando para `google_base`.
pub fn service(base: &Path, google_base: &str) -> Service {
    service_with_os(base, google_base, Arc::new(FakeOs::default()))
}

/// Como [`service`], guardando o sistema de mentira para conferir as chamadas.
pub fn service_with_os(base: &Path, google_base: &str, os: Arc<FakeOs>) -> Service {
    Service::with_backends(
        AppPaths::in_dir(base),
        Arc::new(MemoryStore::default()),
        GoogleBackend {
            endpoints: Endpoints::under(google_base),
            open_browser: false,
        },
        os,
    )
    .expect("serviço de teste")
}
