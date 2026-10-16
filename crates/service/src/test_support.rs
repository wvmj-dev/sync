//! Apoio aos testes: cofre em memória, Google simulado e um systemd de mentira — nunca o
//! keyring, a conta, o systemd nem a sessão reais (AGENTS.md, regra 9).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use syncly_core::paths::AppPaths;
use syncly_gdrive::Endpoints;

use crate::error::{ServiceError, ServiceResult};
use crate::google::GoogleBackend;
use crate::os::OsIntegration;
use crate::secrets::{MemoryStore, SecretStore};
use crate::service::Service;

pub const GOOGLE_CLIENT_JSON: &str =
    r#"{"installed":{"client_id":"1-a.apps.googleusercontent.com","client_secret":"s"}}"#;

/// Executável que os timers de teste apontam (não precisa existir).
pub const FAKE_EXE: &str = "/opt/syncly/syncly-teste";
/// Próximo disparo que os timers de teste informam (segundos Unix).
pub const FAKE_NEXT_RUN: i64 = 1_900_000_000;

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

    /// Todo timer pedido "dispara" num horário fixo; não registra a chamada (é só leitura).
    fn next_runs(&self, timers: &[String]) -> ServiceResult<HashMap<String, DateTime<Utc>>> {
        let at = DateTime::from_timestamp(FAKE_NEXT_RUN, 0).unwrap();
        Ok(timers.iter().map(|t| (t.clone(), at)).collect())
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
    service_with_secrets(base, google_base, Arc::new(MemoryStore::default()), os)
}

/// Como [`service_with_os`], com outro cofre (ex.: [`LockedStore`]).
pub fn service_with_secrets(
    base: &Path,
    google_base: &str,
    secrets: Arc<dyn SecretStore>,
    os: Arc<FakeOs>,
) -> Service {
    Service::with_backends(
        AppPaths::in_dir(base),
        secrets,
        GoogleBackend {
            endpoints: Endpoints::under(google_base),
            open_browser: false,
        },
        os,
    )
    .expect("serviço de teste")
}

/// Cofre bloqueado: tudo falha, como o Secret Service com o pedido de senha recusado.
pub struct LockedStore;

impl SecretStore for LockedStore {
    fn get(&self, _key: &str) -> Result<Option<String>, String> {
        Err("cofre de senhas do sistema indisponível (bloqueado)".into())
    }

    fn set(&self, _key: &str, _value: &str) -> Result<(), String> {
        Err("cofre de senhas do sistema indisponível (bloqueado)".into())
    }

    fn delete(&self, _key: &str) -> Result<(), String> {
        Err("cofre de senhas do sistema indisponível (bloqueado)".into())
    }
}

/// Cofre em memória que anota cada chave lida — prova o que foi (ou não) consultado.
#[derive(Default)]
pub struct RecordingStore {
    inner: MemoryStore,
    pub reads: Mutex<Vec<String>>,
}

impl RecordingStore {
    pub fn reads(&self) -> Vec<String> {
        self.reads.lock().unwrap().clone()
    }
}

impl SecretStore for RecordingStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        self.reads.lock().unwrap().push(key.to_string());
        self.inner.get(key)
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.inner.set(key, value)
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        self.inner.delete(key)
    }
}
