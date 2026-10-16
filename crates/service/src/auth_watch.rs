//! RN-06: observa as chamadas a um provedor que exige login (Google Drive). Se alguma voltar
//! com autorização expirada ou revogada, a conta é marcada como "requer reconexão" — onde quer
//! que o erro apareça (prévia, execução, status, arquivos avulsos, restauração).

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::Utc;
use syncly_core::provider::{
    HashKind, ProviderError, ProviderResult, RemoteEntry, StorageProvider, StorageQuota,
};
use syncly_core::state::StateDb;
use uuid::Uuid;

pub(crate) struct AuthWatch {
    inner: Box<dyn StorageProvider>,
    account_id: Uuid,
    state_db: PathBuf,
}

impl AuthWatch {
    pub(crate) fn new(
        inner: Box<dyn StorageProvider>,
        account_id: Uuid,
        state_db: PathBuf,
    ) -> Self {
        Self {
            inner,
            account_id,
            state_db,
        }
    }

    fn check<T>(&self, result: ProviderResult<T>) -> ProviderResult<T> {
        if matches!(result, Err(ProviderError::AuthExpired)) {
            mark_needs_reconnect(&self.state_db, self.account_id);
        }
        result
    }
}

/// Marca a conta; um problema aqui só vai para o log (o erro original segue seu caminho).
pub(crate) fn mark_needs_reconnect(state_db: &Path, account_id: Uuid) {
    match StateDb::open(state_db).and_then(|db| db.mark_needs_reconnect(account_id, Utc::now())) {
        Ok(()) => tracing::warn!(account = %account_id, "conta precisa ser reconectada"),
        Err(e) => tracing::warn!("conta não marcada para reconexão: {e}"),
    }
}

#[async_trait]
impl StorageProvider for AuthWatch {
    fn id(&self) -> &'static str {
        self.inner.id()
    }

    fn hash_kind(&self) -> HashKind {
        self.inner.hash_kind()
    }

    async fn app_root(&self) -> ProviderResult<RemoteEntry> {
        self.check(self.inner.app_root().await)
    }

    async fn list_children(&self, folder_id: &str) -> ProviderResult<Vec<RemoteEntry>> {
        self.check(self.inner.list_children(folder_id).await)
    }

    async fn get(&self, id: &str) -> ProviderResult<Option<RemoteEntry>> {
        self.check(self.inner.get(id).await)
    }

    async fn list_children_meta(&self, folder_id: &str) -> ProviderResult<Vec<RemoteEntry>> {
        self.check(self.inner.list_children_meta(folder_id).await)
    }

    async fn child_names(&self, folder_id: &str) -> ProviderResult<Vec<String>> {
        self.check(self.inner.child_names(folder_id).await)
    }

    async fn create_folder(&self, parent_id: &str, name: &str) -> ProviderResult<RemoteEntry> {
        self.check(self.inner.create_folder(parent_id, name).await)
    }

    async fn upload_new(
        &self,
        parent_id: &str,
        name: &str,
        local: &Path,
    ) -> ProviderResult<RemoteEntry> {
        self.check(self.inner.upload_new(parent_id, name, local).await)
    }

    async fn upload_replace(&self, id: &str, local: &Path) -> ProviderResult<RemoteEntry> {
        self.check(self.inner.upload_replace(id, local).await)
    }

    async fn download(&self, id: &str, dest: &Path) -> ProviderResult<()> {
        self.check(self.inner.download(id, dest).await)
    }

    async fn rename(&self, id: &str, new_name: &str) -> ProviderResult<RemoteEntry> {
        self.check(self.inner.rename(id, new_name).await)
    }

    async fn trash(&self, id: &str) -> ProviderResult<()> {
        self.check(self.inner.trash(id).await)
    }

    async fn quota(&self) -> ProviderResult<Option<StorageQuota>> {
        self.check(self.inner.quota().await)
    }
}

#[cfg(test)]
mod tests {
    use crate::dto::{JobDraft, SyncDraft};
    use crate::secrets::account_key;
    use crate::test_support::{FakeOs, GOOGLE_CLIENT_JSON, service_with_os};
    use std::sync::Arc;
    use syncly_core::domain::{
        Account, AccountKind, Delivery, RunKind, RunStatus, RunTrigger, Schedule, SyncMode,
    };
    use uuid::Uuid;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn rn06_rn85_login_recusado_marca_a_conta_e_pausa_os_agendamentos() {
        let server = MockServer::start().await;
        // O Google recusa o refresh token (expirado ou revogado) — uma vez só: depois da
        // marca, o app nem tenta a rede.
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(serde_json::json!({"error": "invalid_grant"})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let os = Arc::new(FakeOs::default());
        let svc = service_with_os(&tmp.path().join("app"), &server.uri(), os);
        std::fs::write(svc.paths.google_client_file(), GOOGLE_CLIENT_JSON).unwrap();
        let account = Uuid::new_v4();
        svc.mutate(|c| {
            c.accounts.push(Account {
                id: account,
                label: "eu@gmail.com".into(),
                kind: AccountKind::GoogleDrive {
                    email: Some("eu@gmail.com".into()),
                    root_folder_id: Some("ROOT1".into()),
                },
            });
            Ok(())
        })
        .unwrap();
        svc.secrets.set(&account_key(account), "1//r").unwrap();
        let source = tmp.path().join("Docs");
        std::fs::create_dir_all(&source).unwrap();
        let job = svc
            .save_job(JobDraft {
                id: None,
                name: "Docs".into(),
                enabled: true,
                sources: vec![source],
                exclude: vec![],
                include_sensitive: false,
                sync: Some(SyncDraft {
                    account_id: account,
                    path: "Docs".into(),
                    delivery: Delivery::Files,
                    mode: SyncMode::Upload,
                    schedule: Some(Schedule::Daily {
                        at: "02:30".to_string().try_into().unwrap(),
                    }),
                }),
            })
            .unwrap()
            .id;
        // Já houve um sucesso: o timer está ativo.
        let state = svc.state().unwrap();
        let run = state
            .start_run(job, RunKind::Sync, RunTrigger::Manual)
            .unwrap();
        state
            .finish_run(run, RunStatus::Success, None, None)
            .unwrap();
        drop(state);
        svc.reconcile_schedules().unwrap();
        let timer = svc
            .paths
            .systemd_unit_dir()
            .join(format!("syncly-{job}.timer"));
        assert!(timer.exists());

        // RN-06: o Google recusa → a conta fica marcada, com aviso claro.
        let err = svc.preview_job(job).await.unwrap_err();
        assert!(err.to_string().contains("reconect"), "{err}");
        let accounts = svc.list_accounts().unwrap();
        assert!(accounts[0].needs_reconnect_since.is_some());

        // Daqui em diante, nem tenta a rede (o /token só foi chamado uma vez).
        let err = svc.list_remote(account, "").await.unwrap_err();
        assert_eq!(err.code(), "needs_reconnect");
        let options = crate::service::RunOptions {
            confirm: true,
            trigger: RunTrigger::Manual,
        };
        let err = svc.run_job(job, options, |_| {}).await.unwrap_err();
        assert_eq!(err.code(), "needs_reconnect");

        // RN-85: o agendamento pausa.
        assert!(!timer.exists());
        let summary = svc.list_jobs().unwrap().remove(0);
        assert_eq!(summary.sync.unwrap().schedule_state, "paused");

        // Reconectada (o login em si abre o navegador; aqui, só a parte do app): volta tudo.
        svc.state().unwrap().clear_needs_reconnect(account).unwrap();
        svc.reconcile_schedules().unwrap();
        assert!(timer.exists());
        assert!(
            svc.list_accounts().unwrap()[0]
                .needs_reconnect_since
                .is_none()
        );
    }
}
