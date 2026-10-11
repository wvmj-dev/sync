//! URLs do Google. Configuráveis só para que os testes usem um servidor simulado —
//! nunca a conta real (AGENTS.md, regra 9).

use crate::auth::{AUTH_URL, REVOKE_URL, TOKEN_URL};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoints {
    pub auth: String,
    pub token: String,
    pub revoke: String,
    /// Metadados: `…/drive/v3`.
    pub api: String,
    /// Conteúdo: `…/upload/drive/v3`.
    pub upload: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            auth: AUTH_URL.into(),
            token: TOKEN_URL.into(),
            revoke: REVOKE_URL.into(),
            api: "https://www.googleapis.com/drive/v3".into(),
            upload: "https://www.googleapis.com/upload/drive/v3".into(),
        }
    }
}

impl Endpoints {
    /// Tudo sob uma mesma base (servidor simulado nos testes).
    pub fn under(base: &str) -> Self {
        let base = base.trim_end_matches('/');
        Self {
            auth: format!("{base}/o/oauth2/v2/auth"),
            token: format!("{base}/token"),
            revoke: format!("{base}/revoke"),
            api: format!("{base}/drive/v3"),
            upload: format!("{base}/upload/drive/v3"),
        }
    }

    /// SEG-40: em produção, só HTTPS.
    pub fn all_https(&self) -> bool {
        [
            &self.auth,
            &self.token,
            &self.revoke,
            &self.api,
            &self.upload,
        ]
        .iter()
        .all(|u| u.starts_with("https://"))
    }
}
