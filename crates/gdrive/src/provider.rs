//! `StorageProvider` do Google Drive. Tudo acontece dentro da pasta `Syncly/` (RN-03),
//! encontrada pela marca privada `appProperties.syncly = "root"` (RN-02).

use std::path::Path;

use async_trait::async_trait;
use serde_json::json;
use syncly_core::domain::EntryKind;
use syncly_core::domain::rel_path::validate_name;
use syncly_core::hash::{HashKind, hash_file};
use syncly_core::provider::{
    ProviderError, ProviderResult, RemoteEntry, StorageProvider, StorageQuota,
};
use tokio::sync::OnceCell;

use crate::client::{DriveClient, DriveFile, UploadTarget};
use crate::query::{
    ROOT_FOLDER_NAME, ROOT_PROPERTY_KEY, ROOT_PROPERTY_VALUE, app_root, children_of,
};

const HASH: HashKind = HashKind::Md5;

pub struct GDriveProvider {
    client: DriveClient,
    /// ID da pasta raiz salvo na conta. Se informado e sumir, não criamos outra (RN-08).
    root_hint: Option<String>,
    root: OnceCell<RemoteEntry>,
}

impl GDriveProvider {
    pub fn new(client: DriveClient, root_hint: Option<String>) -> Self {
        Self {
            client,
            root_hint,
            root: OnceCell::new(),
        }
    }

    pub fn client(&self) -> &DriveClient {
        &self.client
    }

    /// RN-02: encontra a pasta raiz pela marca privada ou cria `Syncly/`. Usado ao conectar.
    pub async fn find_or_create_root(&self) -> ProviderResult<RemoteEntry> {
        // Mais antiga primeiro: se dois processos criaram juntos, todos escolhem a mesma.
        let found = self.client.list(&app_root(), Some("createdTime")).await?;
        if let Some(existing) = found.into_iter().next() {
            return Ok(to_entry(existing));
        }
        let created = self
            .client
            .create(json!({
                "name": ROOT_FOLDER_NAME,
                "mimeType": crate::query::FOLDER_MIME,
                "parents": ["root"],
                "appProperties": { ROOT_PROPERTY_KEY: ROOT_PROPERTY_VALUE },
            }))
            .await?;
        Ok(to_entry(created))
    }

    async fn resolve_root(&self) -> ProviderResult<RemoteEntry> {
        let Some(id) = &self.root_hint else {
            return self.find_or_create_root().await;
        };
        match self.client.get_file(id).await? {
            Some(f) if f.is_folder() && !f.trashed => Ok(to_entry(f)),
            _ => Err(ProviderError::NotFound(
                "pasta Syncly/ no Drive (foi para a lixeira?) — reconecte a conta".into(),
            )),
        }
    }

    /// Confere o envio (RN-37). Em arquivo novo com conteúdo divergente, a cópia ruim
    /// vai para a lixeira para não deixar lixo no Drive.
    fn verify(uploaded: &DriveFile, expected: &str) -> ProviderResult<()> {
        match uploaded.md5_checksum.as_deref() {
            Some(got) if got.eq_ignore_ascii_case(expected) => Ok(()),
            _ => Err(ProviderError::ChecksumMismatch),
        }
    }
}

async fn local_hash(path: &Path) -> ProviderResult<String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || hash_file(&path, HASH))
        .await
        .map_err(|e| ProviderError::Other(format!("tarefa interrompida: {e}")))?
        .map_err(ProviderError::Io)
}

fn to_entry(f: DriveFile) -> RemoteEntry {
    let kind = if f.is_folder() {
        EntryKind::Folder
    } else {
        EntryKind::File
    };
    RemoteEntry {
        kind,
        size: f.size.as_deref().and_then(|s| s.parse().ok()),
        // RN-34: Google Docs nativos não têm md5 — ficam `None` e são ignorados.
        hash: f.md5_checksum.map(|h| h.to_ascii_lowercase()),
        modified_at: f.modified_time,
        modified_by: f.last_modifying_user.and_then(|u| u.display_name),
        id: f.id,
        name: f.name,
    }
}

#[async_trait]
impl StorageProvider for GDriveProvider {
    fn id(&self) -> &'static str {
        "google_drive"
    }

    fn hash_kind(&self) -> HashKind {
        HASH
    }

    async fn app_root(&self) -> ProviderResult<RemoteEntry> {
        self.root
            .get_or_try_init(|| self.resolve_root())
            .await
            .cloned()
    }

    async fn list_children(&self, folder_id: &str) -> ProviderResult<Vec<RemoteEntry>> {
        let files = self.client.list(&children_of(folder_id), None).await?;
        Ok(files.into_iter().map(to_entry).collect())
    }

    async fn get(&self, id: &str) -> ProviderResult<Option<RemoteEntry>> {
        Ok(self
            .client
            .get_file(id)
            .await?
            .filter(|f| !f.trashed)
            .map(to_entry))
    }

    async fn create_folder(&self, parent_id: &str, name: &str) -> ProviderResult<RemoteEntry> {
        validate_name(name).map_err(ProviderError::BadRequest)?;
        Ok(to_entry(self.client.create_folder(parent_id, name).await?))
    }

    async fn upload_new(
        &self,
        parent_id: &str,
        name: &str,
        local: &Path,
    ) -> ProviderResult<RemoteEntry> {
        validate_name(name).map_err(ProviderError::BadRequest)?;
        let expected = local_hash(local).await?;
        let uploaded = self
            .client
            .upload(UploadTarget::New { parent_id, name }, local)
            .await?;
        if let Err(e) = Self::verify(&uploaded, &expected) {
            // Nunca exclusão definitiva: a cópia divergente vai para a lixeira (SEG-10).
            let _ = self
                .client
                .update(&uploaded.id, json!({ "trashed": true }))
                .await;
            return Err(e);
        }
        Ok(to_entry(uploaded))
    }

    async fn upload_replace(&self, id: &str, local: &Path) -> ProviderResult<RemoteEntry> {
        let expected = local_hash(local).await?;
        // O Drive guarda a versão anterior no histórico de revisões.
        let uploaded = self
            .client
            .upload(UploadTarget::Replace { id }, local)
            .await?;
        Self::verify(&uploaded, &expected)?;
        Ok(to_entry(uploaded))
    }

    async fn download(&self, id: &str, dest: &Path) -> ProviderResult<()> {
        let meta = self
            .client
            .get_file(id)
            .await?
            .filter(|f| !f.trashed)
            .ok_or_else(|| ProviderError::NotFound(id.to_string()))?;
        if meta.is_folder() {
            return Err(ProviderError::BadRequest(
                "não é possível baixar uma pasta".into(),
            ));
        }
        let expected = meta.md5_checksum.ok_or_else(|| {
            ProviderError::BadRequest(
                "documentos nativos do Google não têm conteúdo para baixar".into(),
            )
        })?;

        // SEG-14: grava num temporário ao lado do destino e só renomeia se o checksum bater.
        let dir = dest.parent().unwrap_or(Path::new("."));
        let tmp = dir.join(format!(".syncly-tmp-{}", uuid::Uuid::new_v4()));
        let mut opts = tokio::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        opts.mode(0o600);
        let mut file = opts.open(&tmp).await?;
        let result = async {
            let (_, got) = self.client.download(id, &mut file, HASH).await?;
            if !got.eq_ignore_ascii_case(&expected) {
                return Err(ProviderError::ChecksumMismatch);
            }
            tokio::fs::rename(&tmp, dest).await?;
            Ok(())
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&tmp).await;
        }
        result
    }

    async fn rename(&self, id: &str, new_name: &str) -> ProviderResult<RemoteEntry> {
        validate_name(new_name).map_err(ProviderError::BadRequest)?;
        Ok(to_entry(
            self.client.update(id, json!({ "name": new_name })).await?,
        ))
    }

    async fn trash(&self, id: &str) -> ProviderResult<()> {
        let root = self.app_root().await?;
        if root.id == id {
            return Err(ProviderError::BadRequest(
                "a pasta raiz não pode ir para a lixeira".into(),
            ));
        }
        self.client.update(id, json!({ "trashed": true })).await?;
        Ok(())
    }

    async fn quota(&self) -> ProviderResult<Option<StorageQuota>> {
        let about = self.client.about().await?;
        Ok(Some(StorageQuota {
            limit: about.limit,
            used: about.usage,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::parse_client;
    use crate::client::Tuning;
    use crate::endpoints::Endpoints;
    use crate::oauth::Secret;
    use std::time::{Duration, Instant};
    use syncly_core::sync::retry::RetryPolicy;
    use wiremock::matchers::{body_partial_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ABC_MD5: &str = "900150983cd24fb0d6963f7d28e17f72";

    fn provider(server: &MockServer, root: Option<&str>, tuning: Tuning) -> GDriveProvider {
        let client = DriveClient::new(
            reqwest::Client::new(),
            parse_client(r#"{"installed":{"client_id":"1-a.apps.googleusercontent.com"}}"#)
                .unwrap(),
            Secret::new("1//r"),
            Endpoints::under(&server.uri()),
            tuning,
        )
        .with_access_token(
            Secret::new("ya29.t"),
            Instant::now() + Duration::from_secs(3600),
        );
        GDriveProvider::new(client, root.map(str::to_owned))
    }

    fn fast() -> Tuning {
        Tuning {
            retry: RetryPolicy {
                max_attempts: 3,
                base_delay: Duration::from_millis(1),
                max_delay: Duration::from_millis(5),
            },
            max_requests_per_second: 10,
            ..Tuning::default()
        }
    }

    fn file(id: &str, name: &str, md5: &str) -> serde_json::Value {
        json!({ "id": id, "name": name, "mimeType": "text/plain", "size": "3",
                "md5Checksum": md5, "modifiedTime": "2026-10-10T12:00:00Z",
                "lastModifyingUser": { "displayName": "Ana Teste" } })
    }

    fn write_abc() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, b"abc").unwrap();
        (dir, p)
    }

    #[tokio::test]
    async fn rn02_cria_pasta_raiz_com_marca_privada() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/drive/v3/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "files": [] })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/drive/v3/files"))
            .and(body_partial_json(json!({
                "name": "Syncly", "parents": ["root"], "appProperties": { "syncly": "root" }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "ROOT1", "name": "Syncly", "mimeType": FOLDER
            })))
            .expect(1)
            .mount(&server)
            .await;
        let p = provider(&server, None, fast());
        assert_eq!(p.app_root().await.unwrap().id, "ROOT1");
        // Em cache: não consulta de novo.
        assert_eq!(p.app_root().await.unwrap().id, "ROOT1");
    }

    const FOLDER: &str = "application/vnd.google-apps.folder";

    #[tokio::test]
    async fn rn08_raiz_na_lixeira_nao_e_recriada() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/drive/v3/files/ROOT1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "ROOT1", "name": "Syncly", "mimeType": FOLDER, "trashed": true
            })))
            .mount(&server)
            .await;
        let p = provider(&server, Some("ROOT1"), fast());
        assert!(matches!(
            p.app_root().await,
            Err(ProviderError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn rn37_envio_verificado_por_checksum() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/upload/drive/v3/files"))
            .and(query_param("uploadType", "multipart"))
            .and(header("authorization", "Bearer ya29.t"))
            .respond_with(ResponseTemplate::new(200).set_body_json(file("F1", "a.txt", ABC_MD5)))
            .expect(1)
            .mount(&server)
            .await;
        let (_d, local) = write_abc();
        let p = provider(&server, None, fast());
        let entry = p.upload_new("PARENT", "a.txt", &local).await.unwrap();
        assert_eq!(entry.id, "F1");
        assert_eq!(entry.hash.as_deref(), Some(ABC_MD5));
        assert_eq!(entry.modified_by.as_deref(), Some("Ana Teste"));
    }

    #[tokio::test]
    async fn seg10_checksum_divergente_manda_copia_para_lixeira() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/upload/drive/v3/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(file("F1", "a.txt", "errado")))
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/drive/v3/files/F1"))
            .and(body_partial_json(json!({ "trashed": true })))
            .respond_with(ResponseTemplate::new(200).set_body_json(file("F1", "a.txt", "errado")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let (_d, local) = write_abc();
        let err = provider(&server, None, fast())
            .upload_new("PARENT", "a.txt", &local)
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::ChecksumMismatch));
    }

    #[tokio::test]
    async fn envio_retomavel_em_blocos() {
        let server = MockServer::start().await;
        let session = format!("{}/upload/session/XYZ", server.uri());
        Mock::given(method("POST"))
            .and(path("/upload/drive/v3/files"))
            .and(query_param("uploadType", "resumable"))
            .respond_with(ResponseTemplate::new(200).insert_header("Location", session.as_str()))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/upload/session/XYZ"))
            .and(header("content-range", "bytes 0-262143/300000"))
            .respond_with(ResponseTemplate::new(308).insert_header("Range", "bytes=0-262143"))
            .expect(1)
            .mount(&server)
            .await;
        let content = vec![7u8; 300_000];
        let md5 = {
            let mut h = syncly_core::hash::ContentHasher::new(HASH);
            h.update(&content);
            h.finish()
        };
        Mock::given(method("PUT"))
            .and(path("/upload/session/XYZ"))
            .and(header("content-range", "bytes 262144-299999/300000"))
            .respond_with(ResponseTemplate::new(200).set_body_json(file("BIG", "big.bin", &md5)))
            .expect(1)
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("big.bin");
        std::fs::write(&local, &content).unwrap();
        let tuning = Tuning {
            simple_upload_limit: 1024,
            chunk_size: 256 * 1024,
            ..fast()
        };
        let entry = provider(&server, None, tuning)
            .upload_new("PARENT", "big.bin", &local)
            .await
            .unwrap();
        assert_eq!(entry.id, "BIG");
    }

    #[tokio::test]
    async fn rn96_tenta_de_novo_apos_limite_de_taxa() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/drive/v3/files/F1"))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/drive/v3/files/F1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(file("F1", "a.txt", ABC_MD5)))
            .mount(&server)
            .await;
        let entry = provider(&server, None, fast()).get("F1").await.unwrap();
        assert_eq!(entry.unwrap().name, "a.txt");
    }

    #[tokio::test]
    async fn download_verificado_e_atomico() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/drive/v3/files/F1"))
            .and(query_param("alt", "media"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"abc".to_vec()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/drive/v3/files/F1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(file("F1", "a.txt", ABC_MD5)))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("baixado.txt");
        provider(&server, None, fast())
            .download("F1", &dest)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"abc");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn download_corrompido_nao_deixa_arquivo() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/drive/v3/files/F1"))
            .and(query_param("alt", "media"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"xyz".to_vec()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/drive/v3/files/F1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(file("F1", "a.txt", ABC_MD5)))
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        let err = provider(&server, None, fast())
            .download("F1", &dir.path().join("x.txt"))
            .await
            .unwrap_err();
        assert!(matches!(err, ProviderError::ChecksumMismatch));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn lista_todas_as_paginas() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/drive/v3/files"))
            .and(query_param("pageToken", "P2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "files": [file("B", "b.txt", ABC_MD5)]
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/drive/v3/files"))
            .and(query_param("q", "'PARENT' in parents and trashed = false"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "files": [file("A", "a.txt", ABC_MD5)], "nextPageToken": "P2"
            })))
            .mount(&server)
            .await;
        let names: Vec<_> = provider(&server, None, fast())
            .list_children("PARENT")
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["a.txt", "b.txt"]);
    }
}
