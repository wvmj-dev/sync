//! Login OAuth 2.0 para app instalado (SEG-01 a SEG-08).
//!
//! 1. Abre a porta local `127.0.0.1:<aleatória>` e monta a URL do Google com PKCE (S256) e `state`.
//! 2. O usuário autoriza no **navegador do sistema** (SEG-02).
//! 3. O Google redireciona para a porta local com o `code`; respondemos uma página estática.
//! 4. Trocamos o `code` (+ `code_verifier`) por tokens e conferimos o escopo concedido.

use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use syncly_core::provider::{ProviderError, ProviderResult};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use url::Url;

use crate::auth::{GoogleClient, SCOPES};
use crate::endpoints::Endpoints;
use crate::error::classify_token_error;
use crate::random;

/// Texto secreto: não aparece em `Debug` nem em logs (SEG-52).
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<oculto>)")
    }
}

#[derive(Debug)]
pub struct Tokens {
    pub access_token: Secret,
    pub expires_at: Instant,
    pub refresh_token: Secret,
}

#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error("não foi possível abrir a porta local para o login: {0}")]
    Listen(std::io::Error),
    #[error("tempo esgotado esperando a autorização no navegador")]
    Timeout,
    #[error("o acesso foi negado na tela do Google")]
    Denied,
    #[error("a permissão de acesso ao Drive não foi concedida — marque-a na tela do Google")]
    ScopeNotGranted,
    #[error("o Google não devolveu um token de atualização — tente conectar de novo")]
    NoRefreshToken,
    #[error(transparent)]
    Provider(#[from] ProviderError),
}

/// RFC 7636: `BASE64URL(SHA256(verifier))`.
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Login em andamento: a porta local já está aberta esperando o retorno do navegador.
pub struct PendingLogin {
    url: String,
    verifier: String,
    state: String,
    redirect_uri: String,
    listener: TcpListener,
}

impl PendingLogin {
    pub async fn start(client: &GoogleClient, endpoints: &Endpoints) -> Result<Self, LoginError> {
        // SEG-01: só a interface de loopback, porta escolhida pelo SO.
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(LoginError::Listen)?;
        let port = listener.local_addr().map_err(LoginError::Listen)?.port();
        let redirect_uri = format!("http://127.0.0.1:{port}");
        let verifier = random::token(48);
        let state = random::token(24);

        let mut url = Url::parse(&endpoints.auth)
            .map_err(|e| ProviderError::Other(format!("URL de autorização inválida: {e}")))?;
        url.query_pairs_mut()
            .append_pair("client_id", &client.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", &SCOPES.join(" "))
            .append_pair("code_challenge", &pkce_challenge(&verifier))
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state)
            // Necessário para receber o refresh token (uso sem o usuário presente).
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent");

        Ok(Self {
            url: url.into(),
            verifier,
            state,
            redirect_uri,
            listener,
        })
    }

    /// URL a abrir no navegador do sistema.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Espera o retorno do navegador e troca o código por tokens.
    pub async fn finish(
        self,
        http: &reqwest::Client,
        client: &GoogleClient,
        endpoints: &Endpoints,
        timeout: Duration,
    ) -> Result<Tokens, LoginError> {
        let code = tokio::time::timeout(timeout, self.wait_for_code())
            .await
            .map_err(|_| LoginError::Timeout)??;
        exchange_code(
            http,
            client,
            endpoints,
            &code,
            &self.verifier,
            &self.redirect_uri,
        )
        .await
    }

    async fn wait_for_code(&self) -> Result<String, LoginError> {
        loop {
            let (mut stream, _) = self.listener.accept().await.map_err(LoginError::Listen)?;
            let Some(target) = read_request_target(&mut stream).await else {
                continue;
            };
            let Ok(url) = Url::parse(&format!("http://127.0.0.1{target}")) else {
                respond(&mut stream, 400, PAGE_ERROR).await;
                continue;
            };
            let mut params: HashMap<String, String> = url.query_pairs().into_owned().collect();
            // Requisições sem resposta do Google (ex.: /favicon.ico) ou com `state`
            // diferente são ignoradas: ninguém além do navegador deste login consegue
            // concluir — nem abortar — o fluxo.
            if params.get("state") != Some(&self.state) {
                respond(&mut stream, 404, PAGE_ERROR).await;
                continue;
            }
            if params.contains_key("error") {
                respond(&mut stream, 200, PAGE_DENIED).await;
                return Err(LoginError::Denied);
            }
            if let Some(code) = params.remove("code") {
                respond(&mut stream, 200, PAGE_OK).await;
                return Ok(code);
            }
            respond(&mut stream, 400, PAGE_ERROR).await;
        }
    }
}

/// Lê só a linha de requisição (`GET /?… HTTP/1.1`), com limite de tamanho e de tempo.
async fn read_request_target(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let read = async {
        while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 8192 {
            let n = stream.read(&mut chunk).await.ok()?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        Some(())
    };
    tokio::time::timeout(Duration::from_secs(5), read)
        .await
        .ok()??;
    let text = String::from_utf8_lossy(&buf);
    let line = text.lines().next()?;
    let mut parts = line.split(' ');
    match (parts.next(), parts.next()) {
        (Some("GET"), Some(target)) if target.starts_with('/') => Some(target.to_string()),
        _ => None,
    }
}

async fn respond(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        _ => "Not Found",
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

// Páginas estáticas: nada da requisição é refletido de volta.
const PAGE_OK: &str = "<!doctype html><meta charset=utf-8><title>Syncly</title>\
<body style='font-family:system-ui;padding:3rem'><h1>Syncly conectado ✓</h1>\
<p>Pode fechar esta aba e voltar para o app.</p>";
const PAGE_DENIED: &str = "<!doctype html><meta charset=utf-8><title>Syncly</title>\
<body style='font-family:system-ui;padding:3rem'><h1>Acesso não autorizado</h1>\
<p>Nada foi alterado. Pode fechar esta aba.</p>";
const PAGE_ERROR: &str = "<!doctype html><meta charset=utf-8><title>Syncly</title>\
<body style='font-family:system-ui;padding:3rem'><h1>Requisição inválida</h1>";

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
    refresh_token: Option<String>,
    #[serde(default)]
    scope: String,
}

async fn exchange_code(
    http: &reqwest::Client,
    client: &GoogleClient,
    endpoints: &Endpoints,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<Tokens, LoginError> {
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("code_verifier", verifier),
        ("client_id", client.client_id.as_str()),
        ("redirect_uri", redirect_uri),
    ];
    if let Some(secret) = client.client_secret() {
        form.push(("client_secret", secret));
    }
    let tokens = post_token(http, &endpoints.token, &form).await?;
    // SEG-08: na tela do Google o usuário pode desmarcar a permissão.
    if !tokens.scope.split(' ').any(|s| SCOPES.contains(&s)) {
        return Err(LoginError::ScopeNotGranted);
    }
    let refresh_token = tokens.refresh_token.ok_or(LoginError::NoRefreshToken)?;
    Ok(Tokens {
        access_token: Secret::new(tokens.access_token),
        expires_at: Instant::now() + Duration::from_secs(tokens.expires_in),
        refresh_token: Secret::new(refresh_token),
    })
}

/// Novo access token a partir do refresh token. `invalid_grant` → [`ProviderError::AuthExpired`]
/// (SEG-07: sem novas tentativas).
pub async fn refresh_access_token(
    http: &reqwest::Client,
    client: &GoogleClient,
    endpoints: &Endpoints,
    refresh_token: &Secret,
) -> ProviderResult<(Secret, Instant)> {
    let mut form = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.expose()),
        ("client_id", client.client_id.as_str()),
    ];
    if let Some(secret) = client.client_secret() {
        form.push(("client_secret", secret));
    }
    let tokens = post_token(http, &endpoints.token, &form).await?;
    Ok((
        Secret::new(tokens.access_token),
        Instant::now() + Duration::from_secs(tokens.expires_in),
    ))
}

/// SEG-06: revoga o acesso no Google. Token já inválido conta como sucesso.
pub async fn revoke(
    http: &reqwest::Client,
    endpoints: &Endpoints,
    token: &Secret,
) -> ProviderResult<()> {
    let response = http
        .post(&endpoints.revoke)
        .form(&[("token", token.expose())])
        .send()
        .await
        .map_err(|e| ProviderError::Network(e.to_string()))?;
    match response.status().as_u16() {
        200 | 400 => Ok(()),
        status => Err(ProviderError::Server {
            status,
            retry_after: None,
        }),
    }
}

async fn post_token(
    http: &reqwest::Client,
    url: &str,
    form: &[(&str, &str)],
) -> ProviderResult<TokenResponse> {
    let response = http
        .post(url)
        .form(form)
        .send()
        .await
        .map_err(|e| ProviderError::Network(e.to_string()))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| ProviderError::Network(e.to_string()))?;
    if status.is_server_error() {
        return Err(ProviderError::Server {
            status: status.as_u16(),
            retry_after: None,
        });
    }
    if !status.is_success() {
        // Só o código de erro é aproveitado; o corpo nunca vai para logs (SEG-52).
        return Err(classify_token_error(&body));
    }
    serde_json::from_str(&body)
        .map_err(|_| ProviderError::Other("resposta inválida do servidor de autenticação".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::parse_client;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client() -> GoogleClient {
        parse_client(
            r#"{"installed":{"client_id":"1-a.apps.googleusercontent.com","client_secret":"s"}}"#,
        )
        .unwrap()
    }

    #[test]
    fn pkce_vetor_da_rfc_7636() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn seg52_segredo_nao_aparece_no_debug() {
        assert_eq!(format!("{:?}", Secret::new("ya29.abc")), "Secret(<oculto>)");
    }

    #[tokio::test]
    async fn seg01_url_de_autorizacao() {
        let login = PendingLogin::start(&client(), &Endpoints::default())
            .await
            .unwrap();
        let url = Url::parse(login.url()).unwrap();
        let q: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(q["scope"], "https://www.googleapis.com/auth/drive.file");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["access_type"], "offline");
        assert!(q["redirect_uri"].starts_with("http://127.0.0.1:"));
        assert_eq!(q["code_challenge"], pkce_challenge(&login.verifier));
    }

    /// Simula o navegador chegando na porta local.
    async fn browser_hits(redirect_uri: &str, query: &str) -> String {
        let addr = redirect_uri.trim_start_matches("http://");
        let mut s = TcpStream::connect(addr).await.unwrap();
        s.write_all(format!("GET /?{query} HTTP/1.1\r\nHost: {addr}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out
    }

    async fn token_server(scope: &str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("grant_type=authorization_code"))
            .and(body_string_contains("code=abc"))
            .and(body_string_contains("code_verifier="))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "ya29.x", "expires_in": 3599,
                "refresh_token": "1//r", "scope": scope, "token_type": "Bearer"
            })))
            .mount(&server)
            .await;
        server
    }

    async fn run_login(scope: &str, queries: Vec<String>) -> Result<Tokens, LoginError> {
        let server = token_server(scope).await;
        let endpoints = Endpoints::under(&server.uri());
        let login = PendingLogin::start(&client(), &endpoints).await.unwrap();
        let redirect = login.redirect_uri.clone();
        let state = login.state.clone();
        let http = reqwest::Client::new();
        let c = client();
        let finish = tokio::spawn(async move {
            login
                .finish(&http, &c, &endpoints, Duration::from_secs(5))
                .await
        });
        for q in queries {
            browser_hits(&redirect, &q.replace("{state}", &state)).await;
        }
        finish.await.unwrap()
    }

    #[tokio::test]
    async fn login_completo_ignora_requisicoes_estranhas() {
        let tokens = run_login(
            "https://www.googleapis.com/auth/drive.file",
            vec![
                "favicon".into(),
                "state=forjado&code=malicioso".into(),
                "state={state}&code=abc&scope=x".into(),
            ],
        )
        .await
        .unwrap();
        assert_eq!(tokens.refresh_token.expose(), "1//r");
    }

    #[tokio::test]
    async fn seg08_escopo_nao_concedido_falha() {
        let err = run_login("openid", vec!["state={state}&code=abc".into()])
            .await
            .unwrap_err();
        assert!(matches!(err, LoginError::ScopeNotGranted));
    }

    #[tokio::test]
    async fn acesso_negado_no_navegador() {
        let err = run_login("x", vec!["state={state}&error=access_denied".into()])
            .await
            .unwrap_err();
        assert!(matches!(err, LoginError::Denied));
    }

    #[tokio::test]
    async fn seg07_refresh_revogado_vira_auth_expired() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(serde_json::json!({"error": "invalid_grant"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let err = refresh_access_token(
            &reqwest::Client::new(),
            &client(),
            &Endpoints::under(&server.uri()),
            &Secret::new("1//r"),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ProviderError::AuthExpired));
    }
}
