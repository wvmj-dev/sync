//! Cliente HTTP da Drive API v3. Concentra o que torna o uso da API seguro e gentil:
//! limitador de taxa (SEG-41), renovação do access token, retentativas com backoff
//! (RN-96) e envio retomável (arquivos grandes continuam de onde pararam).

use std::path::Path;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use syncly_core::hash::{ContentHasher, HashKind};
use syncly_core::provider::{ProviderError, ProviderResult};
use syncly_core::sync::retry::RetryPolicy;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Mutex;

use crate::auth::GoogleClient;
use crate::endpoints::Endpoints;
use crate::error::{classify, parse_body};
use crate::oauth::{Secret, refresh_access_token};
use crate::query::{FILE_FIELDS, FOLDER_MIME, LIST_FIELDS, PAGE_SIZE};
use crate::random;

/// Parâmetros ajustáveis (os testes usam valores pequenos).
#[derive(Debug, Clone)]
pub struct Tuning {
    /// Até este tamanho, envio em uma requisição (multipart).
    pub simple_upload_limit: u64,
    /// Bloco do envio retomável — múltiplo de 256 KiB, exigência do Google.
    pub chunk_size: usize,
    pub retry: RetryPolicy,
    pub max_requests_per_second: u8,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            simple_upload_limit: 5 * 1024 * 1024,
            chunk_size: 8 * 1024 * 1024,
            retry: RetryPolicy::default(),
            max_requests_per_second: 8,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    pub mime_type: String,
    /// O Google manda int64 como string.
    pub size: Option<String>,
    pub md5_checksum: Option<String>,
    pub modified_time: Option<chrono::DateTime<chrono::Utc>>,
    pub last_modifying_user: Option<DriveUser>,
    #[serde(default)]
    pub trashed: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveUser {
    pub display_name: Option<String>,
}

impl DriveFile {
    pub fn is_folder(&self) -> bool {
        self.mime_type == FOLDER_MIME
    }
}

#[derive(Debug, Clone)]
pub struct About {
    pub email: Option<String>,
    pub limit: Option<u64>,
    pub usage: u64,
}

/// Onde o envio vai parar.
pub enum UploadTarget<'a> {
    New { parent_id: &'a str, name: &'a str },
    Replace { id: &'a str },
}

/// Cliente HTTP compartilhado: um só por processo, para reaproveitar conexões (HTTP/2).
pub fn http_client(https_only: bool) -> ProviderResult<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("syncly/", env!("CARGO_PKG_VERSION")))
        .https_only(https_only)
        .connect_timeout(Duration::from_secs(20))
        .read_timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| ProviderError::Other(format!("cliente HTTP: {e}")))
}

pub struct DriveClient {
    http: reqwest::Client,
    endpoints: Endpoints,
    client: GoogleClient,
    refresh_token: Secret,
    access: Mutex<Option<(Secret, Instant)>>,
    limiter: RateLimiter,
    tuning: Tuning,
}

impl DriveClient {
    pub fn new(
        http: reqwest::Client,
        client: GoogleClient,
        refresh_token: Secret,
        endpoints: Endpoints,
        tuning: Tuning,
    ) -> Self {
        Self {
            http,
            endpoints,
            client,
            refresh_token,
            access: Mutex::new(None),
            limiter: RateLimiter::new(tuning.max_requests_per_second),
            tuning,
        }
    }

    /// Reaproveita o access token recém-obtido no login (evita uma renovação imediata).
    pub fn with_access_token(self, token: Secret, expires_at: Instant) -> Self {
        Self {
            access: Mutex::new(Some((token, expires_at))),
            ..self
        }
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }

    pub fn endpoints(&self) -> &Endpoints {
        &self.endpoints
    }

    pub fn refresh_token(&self) -> &Secret {
        &self.refresh_token
    }

    // ---- Infraestrutura ----

    async fn access_token(&self, force_refresh: bool) -> ProviderResult<String> {
        let mut guard = self.access.lock().await;
        if !force_refresh
            && let Some((token, expires_at)) = guard.as_ref()
            && *expires_at > Instant::now() + Duration::from_secs(60)
        {
            return Ok(token.expose().to_string());
        }
        let (token, expires_at) = refresh_access_token(
            &self.http,
            &self.client,
            &self.endpoints,
            &self.refresh_token,
        )
        .await?;
        let value = token.expose().to_string();
        *guard = Some((token, expires_at));
        Ok(value)
    }

    /// Uma tentativa: limitador + token + envio. Em 401, renova o token uma vez.
    async fn send_once<F>(&self, build: &F) -> ProviderResult<reqwest::Response>
    where
        F: Fn(&reqwest::Client) -> reqwest::RequestBuilder,
    {
        let mut refreshed = false;
        loop {
            self.limiter.wait().await;
            let token = self.access_token(refreshed).await?;
            let response = build(&self.http)
                .bearer_auth(&token)
                .send()
                .await
                .map_err(network)?;
            let status = response.status().as_u16();
            if status == 401 && !refreshed {
                refreshed = true;
                continue;
            }
            if response.status().is_success() || status == 308 {
                return Ok(response);
            }
            return Err(error_from(response).await);
        }
    }

    /// Envio com retentativas para erros transitórios (RN-96). Erros de item ou de
    /// conta voltam na hora (RN-97, RN-98).
    async fn send<F>(&self, build: F) -> ProviderResult<reqwest::Response>
    where
        F: Fn(&reqwest::Client) -> reqwest::RequestBuilder,
    {
        let mut attempt = 0;
        loop {
            attempt += 1;
            match self.send_once(&build).await {
                Ok(response) => return Ok(response),
                Err(err) => match self.tuning.retry.next_delay(attempt, &err, random::unit()) {
                    Some(delay) => {
                        tracing::debug!(attempt, ?delay, "erro transitório na Drive API: {err}");
                        tokio::time::sleep(delay).await;
                    }
                    None => return Err(err),
                },
            }
        }
    }

    async fn json<T: DeserializeOwned>(response: reqwest::Response) -> ProviderResult<T> {
        let body = response.bytes().await.map_err(network)?;
        serde_json::from_slice(&body)
            .map_err(|e| ProviderError::Other(format!("resposta inesperada da Drive API: {e}")))
    }

    // ---- Metadados ----

    /// `None` se não existir (404).
    pub async fn get_file(&self, id: &str) -> ProviderResult<Option<DriveFile>> {
        check_id(id)?;
        let url = format!("{}/files/{id}", self.endpoints.api);
        match self
            .send(|h| h.get(&url).query(&[("fields", FILE_FIELDS)]))
            .await
        {
            Ok(response) => Ok(Some(Self::json(response).await?)),
            Err(ProviderError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Lista todas as páginas de uma consulta.
    pub async fn list(&self, q: &str, order_by: Option<&str>) -> ProviderResult<Vec<DriveFile>> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Page {
            #[serde(default)]
            files: Vec<DriveFile>,
            next_page_token: Option<String>,
        }
        let url = format!("{}/files", self.endpoints.api);
        let page_size = PAGE_SIZE.to_string();
        let mut out = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let response = self
                .send(|h| {
                    let mut req = h.get(&url).query(&[
                        ("q", q),
                        ("fields", LIST_FIELDS),
                        ("pageSize", page_size.as_str()),
                        ("spaces", "drive"),
                    ]);
                    if let Some(order) = order_by {
                        req = req.query(&[("orderBy", order)]);
                    }
                    if let Some(t) = &token {
                        req = req.query(&[("pageToken", t.as_str())]);
                    }
                    req
                })
                .await?;
            let page: Page = Self::json(response).await?;
            out.extend(page.files);
            match page.next_page_token {
                Some(next) => token = Some(next),
                None => return Ok(out),
            }
        }
    }

    pub async fn create(&self, metadata: Value) -> ProviderResult<DriveFile> {
        let url = format!("{}/files", self.endpoints.api);
        let response = self
            .send(|h| {
                h.post(&url)
                    .query(&[("fields", FILE_FIELDS)])
                    .json(&metadata)
            })
            .await?;
        Self::json(response).await
    }

    pub async fn create_folder(&self, parent_id: &str, name: &str) -> ProviderResult<DriveFile> {
        check_id(parent_id)?;
        self.create(json!({ "name": name, "mimeType": FOLDER_MIME, "parents": [parent_id] }))
            .await
    }

    /// Altera metadados (nome, `trashed`…). Nunca existe `DELETE` aqui (SEG-10).
    pub async fn update(&self, id: &str, metadata: Value) -> ProviderResult<DriveFile> {
        check_id(id)?;
        let url = format!("{}/files/{id}", self.endpoints.api);
        let response = self
            .send(|h| {
                h.patch(&url)
                    .query(&[("fields", FILE_FIELDS)])
                    .json(&metadata)
            })
            .await?;
        Self::json(response).await
    }

    pub async fn about(&self) -> ProviderResult<About> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Raw {
            user: Option<RawUser>,
            storage_quota: Option<RawQuota>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct RawUser {
            email_address: Option<String>,
        }
        #[derive(Deserialize)]
        struct RawQuota {
            limit: Option<String>,
            usage: Option<String>,
        }
        let url = format!("{}/about", self.endpoints.api);
        let response = self
            .send(|h| {
                h.get(&url)
                    .query(&[("fields", "user(emailAddress),storageQuota(limit,usage)")])
            })
            .await?;
        let raw: Raw = Self::json(response).await?;
        let quota = raw.storage_quota;
        Ok(About {
            email: raw.user.and_then(|u| u.email_address),
            limit: quota
                .as_ref()
                .and_then(|q| q.limit.as_deref())
                .and_then(|l| l.parse().ok()),
            usage: quota
                .as_ref()
                .and_then(|q| q.usage.as_deref())
                .and_then(|u| u.parse().ok())
                .unwrap_or(0),
        })
    }

    // ---- Conteúdo ----

    /// Envia o conteúdo de `path`. Pequenos: uma requisição; grandes: retomável.
    pub async fn upload(&self, target: UploadTarget<'_>, path: &Path) -> ProviderResult<DriveFile> {
        let size = tokio::fs::metadata(path).await?.len();
        let (method, url, metadata) = match target {
            UploadTarget::New { parent_id, name } => {
                check_id(parent_id)?;
                (
                    reqwest::Method::POST,
                    format!("{}/files", self.endpoints.upload),
                    json!({ "name": name, "parents": [parent_id] }),
                )
            }
            UploadTarget::Replace { id } => {
                check_id(id)?;
                (
                    reqwest::Method::PATCH,
                    format!("{}/files/{id}", self.endpoints.upload),
                    json!({}),
                )
            }
        };
        if size <= self.tuning.simple_upload_limit {
            self.upload_multipart(method, &url, &metadata, path).await
        } else {
            self.upload_resumable(method, &url, &metadata, path, size)
                .await
        }
    }

    async fn upload_multipart(
        &self,
        method: reqwest::Method,
        url: &str,
        metadata: &Value,
        path: &Path,
    ) -> ProviderResult<DriveFile> {
        let content = tokio::fs::read(path).await?;
        let boundary = format!("syncly-{}", random::token(18));
        let mut body = Vec::with_capacity(content.len() + 512);
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n\
                 --{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(&content);
        body.extend_from_slice(format!("\r\n--{boundary}--").as_bytes());
        let content_type = format!("multipart/related; boundary={boundary}");

        let response = self
            .send(|h| {
                h.request(method.clone(), url)
                    .query(&[("uploadType", "multipart"), ("fields", FILE_FIELDS)])
                    .header(reqwest::header::CONTENT_TYPE, &content_type)
                    .body(body.clone())
            })
            .await?;
        Self::json(response).await
    }

    async fn upload_resumable(
        &self,
        method: reqwest::Method,
        url: &str,
        metadata: &Value,
        path: &Path,
        size: u64,
    ) -> ProviderResult<DriveFile> {
        let size_header = size.to_string();
        let response = self
            .send(|h| {
                h.request(method.clone(), url)
                    .query(&[("uploadType", "resumable"), ("fields", FILE_FIELDS)])
                    .header("X-Upload-Content-Type", "application/octet-stream")
                    .header("X-Upload-Content-Length", &size_header)
                    .json(metadata)
            })
            .await?;
        let session = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
            .ok_or_else(|| ProviderError::Other("Drive não devolveu a sessão de envio".into()))?;

        let mut file = tokio::fs::File::open(path).await?;
        let mut buf = vec![0u8; self.tuning.chunk_size];
        let mut offset = 0u64;
        let mut failures = 0;
        loop {
            let len = (size - offset).min(buf.len() as u64) as usize;
            file.seek(std::io::SeekFrom::Start(offset)).await?;
            file.read_exact(&mut buf[..len]).await?;
            let range = format!("bytes {offset}-{}/{size}", offset + len as u64 - 1);
            let chunk = &buf[..len];
            let outcome = self
                .send_once(&|h: &reqwest::Client| {
                    h.put(&session)
                        .header(reqwest::header::CONTENT_RANGE, &range)
                        .body(chunk.to_vec())
                })
                .await;
            let response = match outcome {
                Ok(r) => r,
                Err(err) => {
                    failures += 1;
                    let delay = self
                        .tuning
                        .retry
                        .next_delay(failures, &err, random::unit())
                        .ok_or(err)?;
                    tokio::time::sleep(delay).await;
                    // Pergunta ao Google quanto já chegou e continua dali.
                    let status_range = format!("bytes */{size}");
                    self.send(|h| {
                        h.put(&session)
                            .header(reqwest::header::CONTENT_RANGE, &status_range)
                            .header(reqwest::header::CONTENT_LENGTH, "0")
                    })
                    .await?
                }
            };
            if response.status().as_u16() != 308 {
                return Self::json(response).await;
            }
            offset = received_bytes(&response);
        }
    }

    /// Baixa o conteúdo para `out`, calculando o hash no caminho. Retorna (bytes, hash).
    pub async fn download(
        &self,
        id: &str,
        out: &mut tokio::fs::File,
        kind: HashKind,
    ) -> ProviderResult<(u64, String)> {
        check_id(id)?;
        let url = format!("{}/files/{id}", self.endpoints.api);
        let mut response = self
            .send(|h| h.get(&url).query(&[("alt", "media")]))
            .await?;
        let mut hasher = ContentHasher::new(kind);
        let mut total = 0u64;
        while let Some(chunk) = response.chunk().await.map_err(network)? {
            hasher.update(&chunk);
            out.write_all(&chunk).await?;
            total += chunk.len() as u64;
        }
        out.flush().await?;
        out.sync_all().await?;
        Ok((total, hasher.finish()))
    }
}

/// `Range: bytes=0-N` → próximo byte a enviar (N+1). Sem cabeçalho: nada chegou.
fn received_bytes(response: &reqwest::Response) -> u64 {
    response
        .headers()
        .get(reqwest::header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('-').next())
        .and_then(|end| end.parse::<u64>().ok())
        .map_or(0, |end| end + 1)
}

/// IDs do Drive só têm letras, números, `-` e `_`. Qualquer outra coisa poderia
/// alterar o caminho da URL (SEG-25).
pub fn check_id(id: &str) -> ProviderResult<()> {
    let valid = !id.is_empty()
        && id.len() <= 256
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if valid || id == "root" {
        Ok(())
    } else {
        Err(ProviderError::BadRequest(format!(
            "ID do Drive inválido: {id:?}"
        )))
    }
}

fn network(err: reqwest::Error) -> ProviderError {
    ProviderError::Network(err.to_string())
}

async fn error_from(response: reqwest::Response) -> ProviderError {
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs);
    let body = response.text().await.unwrap_or_default();
    let (reason, message) = parse_body(&body);
    classify(status, reason.as_deref(), &message, retry_after)
}

/// Espaça as requisições para no máximo `rps` por segundo neste processo (SEG-41).
struct RateLimiter {
    interval: Duration,
    next: Mutex<tokio::time::Instant>,
}

impl RateLimiter {
    fn new(rps: u8) -> Self {
        Self {
            interval: Duration::from_secs(1) / u32::from(rps.max(1)),
            next: Mutex::new(tokio::time::Instant::now()),
        }
    }

    async fn wait(&self) {
        let slot = {
            let mut next = self.next.lock().await;
            let slot = (*next).max(tokio::time::Instant::now());
            *next = slot + self.interval;
            slot
        };
        tokio::time::sleep_until(slot).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seg25_ids_validos() {
        assert!(check_id("1AbC-_9").is_ok());
        assert!(check_id("root").is_ok());
        for bad in ["", "../x", "a/b", "a?b", "a b", "a%2F"] {
            assert!(check_id(bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn seg41_limitador_espaca_requisicoes() {
        let limiter = RateLimiter::new(10);
        let start = Instant::now();
        for _ in 0..4 {
            limiter.wait().await;
        }
        // 4 requisições a 10/s: pelo menos 300 ms entre a primeira e a última.
        assert!(start.elapsed() >= Duration::from_millis(290));
    }
}
