//! Cofre de segredos (SEG-04). Em produção: keyring do sistema (Secret Service no Linux —
//! GNOME Keyring/KWallet). Nos testes: memória — nunca o cofre real do usuário.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

use uuid::Uuid;

/// Erros já com mensagem para o usuário; nunca contêm o segredo.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, value: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}

/// Serviço usado no keyring.
pub const SERVICE: &str = "syncly";

/// Entrada do refresh token de uma conta: `account:<uuid>`.
pub fn account_key(id: Uuid) -> String {
    format!("account:{id}")
}

/// Entrada de uma chave privada da criptografia (SEG-61): `age-key:<impressão digital>`.
pub fn encryption_key(fingerprint: &str) -> String {
    format!("age-key:{fingerprint}")
}

pub struct KeyringStore;

fn keyring_error(e: keyring::Error) -> String {
    format!("cofre de senhas do sistema indisponível ({e}) — o keyring está desbloqueado?")
}

impl SecretStore for KeyringStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        let entry = keyring::Entry::new(SERVICE, key).map_err(keyring_error)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keyring_error(e)),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        keyring::Entry::new(SERVICE, key)
            .and_then(|e| e.set_password(value))
            .map_err(keyring_error)
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        let entry = keyring::Entry::new(SERVICE, key).map_err(keyring_error)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keyring_error(e)),
        }
    }
}

#[derive(Default)]
pub struct MemoryStore {
    values: Mutex<HashMap<String, String>>,
}

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        let values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(values.get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.remove(key);
        Ok(())
    }
}
