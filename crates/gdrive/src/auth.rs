//! Configuração OAuth (SEG-01 a SEG-07).

use std::fmt;
use std::path::Path;

use serde::Deserialize;

/// SEG-03: escopo único. O Google só deixa o app acessar arquivos que ele criou.
pub const SCOPES: &[&str] = &["https://www.googleapis.com/auth/drive.file"];
pub const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// SEG-06
pub const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
/// SEG-04: serviço no keyring; entradas `account:<uuid>`.
pub const KEYRING_SERVICE: &str = "syncly";
/// SEG-01: o servidor de loopback espera no máximo isto pelo retorno do navegador.
pub const LOGIN_TIMEOUT_SECS: u64 = 300;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ClientConfigError {
    #[error("arquivo do cliente OAuth não encontrado em {0} — veja docs/CONFIGURAR_GOOGLE.md")]
    Missing(String),
    #[error("não foi possível ler o cliente OAuth: {0}")]
    Unreadable(String),
    #[error("JSON do cliente OAuth inválido: {0}")]
    Invalid(String),
    #[error(
        "o cliente OAuth é do tipo 'web'; crie um cliente do tipo 'App para computador' (SEG-05)"
    )]
    WebClient,
}

/// Credenciais do cliente "App para computador" baixadas do Google Cloud.
/// O `Debug` não revela o segredo (SEG-52).
#[derive(Clone, PartialEq, Eq)]
pub struct GoogleClient {
    pub client_id: String,
    client_secret: Option<String>,
}

impl GoogleClient {
    pub fn client_secret(&self) -> Option<&str> {
        self.client_secret.as_deref()
    }
}

impl fmt::Debug for GoogleClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GoogleClient")
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "<oculto>"),
            )
            .finish()
    }
}

#[derive(Deserialize)]
struct ClientFile {
    installed: Option<InstalledSection>,
    web: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct InstalledSection {
    client_id: String,
    client_secret: Option<String>,
}

pub fn parse_client(json: &str) -> Result<GoogleClient, ClientConfigError> {
    let file: ClientFile =
        serde_json::from_str(json).map_err(|e| ClientConfigError::Invalid(e.to_string()))?;
    let installed = match (file.installed, file.web) {
        (Some(i), _) => i,
        (None, Some(_)) => return Err(ClientConfigError::WebClient),
        (None, None) => {
            return Err(ClientConfigError::Invalid(
                "seção 'installed' ausente".into(),
            ));
        }
    };
    let id = installed.client_id.trim();
    if !id.ends_with(".apps.googleusercontent.com") || id.chars().any(char::is_whitespace) {
        return Err(ClientConfigError::Invalid("client_id inesperado".into()));
    }
    Ok(GoogleClient {
        client_id: id.to_string(),
        client_secret: installed.client_secret.filter(|s| !s.trim().is_empty()),
    })
}

/// Lê o `google-client.json`. Se estiver legível por outros usuários, corrige
/// para `0600` (SEG-05) antes de usar.
pub fn load_client(path: &Path) -> Result<GoogleClient, ClientConfigError> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            ClientConfigError::Missing(path.display().to_string())
        } else {
            ClientConfigError::Unreadable(e.to_string())
        }
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta =
            std::fs::metadata(path).map_err(|e| ClientConfigError::Unreadable(e.to_string()))?;
        if meta.permissions().mode() & 0o077 != 0 {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| ClientConfigError::Unreadable(e.to_string()))?;
        }
    }
    parse_client(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESKTOP: &str = r#"{"installed":{"client_id":"123-abc.apps.googleusercontent.com",
        "project_id":"syncly-pessoal","auth_uri":"https://accounts.google.com/o/oauth2/auth",
        "token_uri":"https://oauth2.googleapis.com/token","client_secret":"GOCSPX-segredo",
        "redirect_uris":["http://localhost"]}}"#;

    #[test]
    fn aceita_cliente_desktop() {
        let c = parse_client(DESKTOP).unwrap();
        assert_eq!(c.client_id, "123-abc.apps.googleusercontent.com");
        assert_eq!(c.client_secret(), Some("GOCSPX-segredo"));
    }

    #[test]
    fn seg05_recusa_cliente_web() {
        let web = r#"{"web":{"client_id":"x.apps.googleusercontent.com"}}"#;
        assert_eq!(parse_client(web), Err(ClientConfigError::WebClient));
    }

    #[test]
    fn seg52_debug_nao_mostra_segredo() {
        let c = parse_client(DESKTOP).unwrap();
        let shown = format!("{c:?}");
        assert!(!shown.contains("GOCSPX"));
    }

    #[cfg(unix)]
    #[test]
    fn seg05_corrige_permissao_do_arquivo() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("google-client.json");
        std::fs::write(&path, DESKTOP).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        load_client(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn arquivo_ausente_tem_mensagem_util() {
        let err = load_client(Path::new("/nao/existe.json")).unwrap_err();
        assert!(err.to_string().contains("CONFIGURAR_GOOGLE"));
    }
}
