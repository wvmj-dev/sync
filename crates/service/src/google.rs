//! Conexão de contas Google (RN-01, RN-02, RN-06; SEG-01 a SEG-08).

use std::time::Duration;

use syncly_core::CoreError;
use syncly_core::domain::{Account, AccountKind};
use syncly_core::provider::StorageProvider;
use syncly_gdrive::auth::{GoogleClient, LOGIN_TIMEOUT_SECS, load_client};
use syncly_gdrive::{
    DriveClient, Endpoints, GDriveProvider, HttpClient, LoginError, PendingLogin, Secret, Tuning,
    http_client, oauth,
};
use uuid::Uuid;

use crate::dto::AccountSummary;
use crate::error::{ServiceError, ServiceResult};
use crate::secrets::account_key;
use crate::service::{Service, account_summary};

/// Como falar com o Google. Os testes trocam por um servidor simulado.
#[derive(Debug, Clone)]
pub struct GoogleBackend {
    pub endpoints: Endpoints,
    /// Abre o navegador do sistema no login (SEG-02). Desligado nos testes.
    pub open_browser: bool,
}

impl Default for GoogleBackend {
    fn default() -> Self {
        Self {
            endpoints: Endpoints::default(),
            open_browser: true,
        }
    }
}

fn login_error(err: LoginError) -> ServiceError {
    match err {
        LoginError::Provider(e) => CoreError::Provider(e).into(),
        other => ServiceError::Unavailable(other.to_string()),
    }
}

impl Service {
    pub(crate) fn http(&self) -> ServiceResult<HttpClient> {
        if let Some(client) = self.http.get() {
            return Ok(client.clone());
        }
        // SEG-40: só HTTPS com os endpoints reais.
        let client = http_client(self.google.endpoints.all_https()).map_err(CoreError::from)?;
        Ok(self.http.get_or_init(|| client).clone())
    }

    pub(crate) fn google_client(&self) -> ServiceResult<GoogleClient> {
        load_client(&self.paths.google_client_file())
            .map_err(|e| ServiceError::Unavailable(e.to_string()))
    }

    pub(crate) fn tuning(&self) -> ServiceResult<Tuning> {
        Ok(Tuning {
            max_requests_per_second: self.load()?.settings.max_requests_per_second,
            ..Tuning::default()
        })
    }

    pub(crate) async fn secret_get(&self, key: String) -> ServiceResult<Option<String>> {
        let store = self.secrets.clone();
        tokio::task::spawn_blocking(move || store.get(&key))
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?
            .map_err(ServiceError::Unavailable)
    }

    async fn secret_set(&self, key: String, value: String) -> ServiceResult<()> {
        let store = self.secrets.clone();
        tokio::task::spawn_blocking(move || store.set(&key, &value))
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?
            .map_err(ServiceError::Unavailable)
    }

    pub(crate) async fn secret_delete(&self, key: String) -> ServiceResult<()> {
        let store = self.secrets.clone();
        tokio::task::spawn_blocking(move || store.delete(&key))
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?
            .map_err(ServiceError::Unavailable)
    }

    /// Provedor do Drive para uma conta já conectada.
    pub(crate) async fn google_provider(
        &self,
        account: &Account,
        root_folder_id: Option<String>,
    ) -> ServiceResult<Box<dyn StorageProvider>> {
        let token = self
            .secret_get(account_key(account.id))
            .await?
            .ok_or_else(|| {
                ServiceError::Unavailable(format!(
                    "a conta '{}' precisa ser reconectada (credencial não encontrada no cofre)",
                    account.label
                ))
            })?;
        let drive = DriveClient::new(
            self.http()?,
            self.google_client()?,
            Secret::new(token),
            self.google.endpoints.clone(),
            self.tuning()?,
        );
        Ok(Box::new(GDriveProvider::new(drive, root_folder_id)))
    }

    /// Conecta (ou reconecta) uma conta Google. `show_url` recebe a URL de login — a CLI
    /// a imprime caso o navegador não abra sozinho.
    pub async fn connect_google(
        &self,
        show_url: impl FnOnce(&str) + Send,
    ) -> ServiceResult<AccountSummary> {
        let client = self.google_client()?;
        let endpoints = self.google.endpoints.clone();
        let login = PendingLogin::start(&client, &endpoints)
            .await
            .map_err(login_error)?;
        show_url(login.url());
        if self.google.open_browser
            && let Err(e) = open::that_detached(login.url())
        {
            tracing::warn!("não foi possível abrir o navegador: {e}");
        }
        let http = self.http()?;
        let tokens = login
            .finish(
                &http,
                &client,
                &endpoints,
                Duration::from_secs(LOGIN_TIMEOUT_SECS),
            )
            .await
            .map_err(login_error)?;

        let drive = DriveClient::new(
            http,
            client,
            tokens.refresh_token.clone(),
            endpoints,
            self.tuning()?,
        )
        .with_access_token(tokens.access_token, tokens.expires_at);
        let provider = GDriveProvider::new(drive, None);
        let about = provider.client().about().await.map_err(CoreError::from)?;
        let root = provider
            .find_or_create_root()
            .await
            .map_err(CoreError::from)?;

        // RN-06: mesmo e-mail = reconexão da conta existente (as tarefas continuam valendo).
        let existing = self.load()?.accounts.into_iter().find(|a| {
            matches!(&a.kind, AccountKind::GoogleDrive { email: Some(e), .. }
                if about.email.as_ref() == Some(e))
        });
        let id = existing.as_ref().map_or_else(Uuid::new_v4, |a| a.id);
        let account = Account {
            id,
            label: existing
                .map(|a| a.label)
                .or_else(|| about.email.clone())
                .unwrap_or_else(|| "Google Drive".into()),
            kind: AccountKind::GoogleDrive {
                email: about.email,
                root_folder_id: Some(root.id),
            },
        };

        // SEG-04: o refresh token vai para o cofre antes da conta ir para a config.
        self.secret_set(account_key(id), tokens.refresh_token.expose().to_string())
            .await?;
        let saved = account.clone();
        let result = self.mutate(move |config| {
            match config.accounts.iter_mut().find(|a| a.id == id) {
                Some(slot) => *slot = account,
                None => config.accounts.push(account),
            }
            Ok(())
        });
        if let Err(e) = result {
            let _ = self.secret_delete(account_key(id)).await;
            return Err(e);
        }
        tracing::info!(account = %id, "conta Google conectada");
        Ok(account_summary(&self.load()?, &saved))
    }

    /// SEG-06: revoga no Google (melhor esforço) e apaga do cofre.
    pub(crate) async fn forget_google_credentials(&self, account_id: Uuid) -> ServiceResult<()> {
        if let Some(token) = self.secret_get(account_key(account_id)).await? {
            let http = self.http()?;
            if let Err(e) = oauth::revoke(&http, &self.google.endpoints, &Secret::new(token)).await
            {
                tracing::warn!("não foi possível revogar o acesso no Google: {e}");
            }
        }
        self.secret_delete(account_key(account_id)).await
    }
}
