//! Transferências avulsas: listar, enviar e baixar um arquivo (RN-104 a RN-106), baixar e
//! descompactar (RN-108) e enviar compactado (RN-109). Funcionam com qualquer provedor —
//! Drive ou pasta local.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use syncly_core::CoreError;
use syncly_core::archive::{self, ArchiveFormat, ArchiveProgress, CreateRequest};
use syncly_core::domain::rel_path::{join_str, validate_name};
use syncly_core::domain::{EntryKind, RelPath};
use syncly_core::filters::Filter;
use syncly_core::fsutil::unique_file_path;
use syncly_core::provider::{RemoteEntry, StorageProvider};
use syncly_core::sync::remote_tree::resolve_path;
use uuid::Uuid;

use crate::archive::{
    ExtractResult, Slot, absolute_dir, archive_stem, blocking, check_inputs, ensure_space,
    extract_new_folder, total_size,
};
use crate::delivery::{TMP_PREFIX, TempFile};
use crate::dto::{RemoteItem, UploadCompressedRequest};
use crate::error::{ServiceError, ServiceResult};
use crate::service::{Service, quota_shortfall};

/// Temporários das operações avulsas (a vaga única de compactação os protege).
const STAGING: &str = "avulsos";

impl Service {
    /// Conta por ID, nome ou e-mail. Sem chave: a única conta existente.
    pub fn find_account_id(&self, key: Option<&str>) -> ServiceResult<Uuid> {
        let config = self.load()?;
        let available = || {
            let labels: Vec<_> = config.accounts.iter().map(|a| a.label.as_str()).collect();
            let list = if labels.is_empty() {
                "nenhuma conta cadastrada".to_string()
            } else {
                format!("contas: {}", labels.join(", "))
            };
            format!("{list}. Para conectar o Google Drive: syncly google connect")
        };
        let Some(key) = key.map(str::trim) else {
            return match config.accounts.as_slice() {
                [only] => Ok(only.id),
                [] => Err(ServiceError::Invalid(available())),
                _ => Err(ServiceError::Invalid(format!(
                    "escolha a conta com --account — {}",
                    available()
                ))),
            };
        };
        config
            .accounts
            .iter()
            .find(|a| {
                a.id.to_string() == key
                    || a.label.eq_ignore_ascii_case(key)
                    || matches!(&a.kind, syncly_core::domain::AccountKind::GoogleDrive { email: Some(e), .. } if e.eq_ignore_ascii_case(key))
            })
            .map(|a| a.id)
            .ok_or_else(|| {
                ServiceError::Invalid(format!("conta '{key}' não encontrada — {}", available()))
            })
    }

    async fn provider_by_id(&self, account_id: Uuid) -> ServiceResult<Box<dyn StorageProvider>> {
        let account = self
            .load()?
            .account(account_id)
            .cloned()
            .ok_or(ServiceError::NotFound("conta"))?;
        self.provider_for(&account).await
    }

    /// Conteúdo de uma pasta dentro da raiz do app (`""` = a própria raiz).
    pub async fn list_remote(
        &self,
        account_id: Uuid,
        path: &str,
    ) -> ServiceResult<Vec<RemoteItem>> {
        let path = RelPath::parse(path).map_err(ServiceError::Invalid)?;
        let provider = self.provider_by_id(account_id).await?;
        let folder = resolve_folder(provider.as_ref(), &path, false).await?;
        let mut items: Vec<RemoteItem> = provider
            .list_children(&folder.id)
            .await
            .map_err(CoreError::from)?
            .into_iter()
            .filter(|e| validate_name(&e.name).is_ok())
            .map(|e| to_item(path.as_str(), e))
            .collect();
        items.sort_by(|a, b| {
            (a.kind != "folder", a.name.to_lowercase())
                .cmp(&(b.kind != "folder", b.name.to_lowercase()))
        });
        Ok(items)
    }

    /// Envia um arquivo local para `remote_dir` (criada se preciso). Nunca sobrescreve
    /// nem duplica (RN-104) e respeita os filtros de arquivos sensíveis (RN-105).
    pub async fn upload_file(
        &self,
        account_id: Uuid,
        local: &Path,
        remote_dir: &str,
    ) -> ServiceResult<RemoteItem> {
        if !local.is_absolute() || !local.is_file() {
            return Err(ServiceError::Invalid(format!(
                "'{}' não é um arquivo existente",
                local.display()
            )));
        }
        let name = local
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| validate_name(n).is_ok())
            .ok_or_else(|| ServiceError::Invalid("nome de arquivo inválido".into()))?
            .to_string();
        let filter = Filter::new(&[], false).map_err(ServiceError::Invalid)?;
        if filter.is_excluded(Path::new(&name), false) {
            return Err(ServiceError::Invalid(format!(
                "'{name}' é um arquivo sensível ou temporário e não é enviado (RN-105)"
            )));
        }

        let dir = RelPath::parse(remote_dir).map_err(ServiceError::Invalid)?;
        let provider = self.provider_by_id(account_id).await?;
        let folder = resolve_folder(provider.as_ref(), &dir, true).await?;
        ensure_free_name(provider.as_ref(), &folder.id, &name, &dir).await?;
        let entry = provider
            .upload_new(&folder.id, &name, local)
            .await
            .map_err(CoreError::from)?;
        tracing::info!(account = %account_id, "arquivo enviado");
        Ok(to_item(dir.as_str(), entry))
    }

    /// Baixa `remote_path` para dentro de `local_dir`. Nunca sobrescreve: se o nome já
    /// existir, usa `nome (2).ext` (RN-106). Retorna o caminho gravado.
    pub async fn download_file(
        &self,
        account_id: Uuid,
        remote_path: &str,
        local_dir: &Path,
    ) -> ServiceResult<PathBuf> {
        if !local_dir.is_absolute() || !local_dir.is_dir() {
            return Err(ServiceError::Invalid(format!(
                "'{}' não é uma pasta existente",
                local_dir.display()
            )));
        }
        let path = RelPath::parse(remote_path).map_err(ServiceError::Invalid)?;
        let name = path
            .components()
            .last()
            .ok_or_else(|| ServiceError::Invalid("informe o caminho de um arquivo".into()))?
            .to_string();
        let parent = RelPath::parse(syncly_core::domain::rel_path::parent_str(path.as_str()))
            .map_err(ServiceError::Invalid)?;

        let provider = self.provider_by_id(account_id).await?;
        let folder = resolve_folder(provider.as_ref(), &parent, false).await?;
        let entry = single_child(provider.as_ref(), &folder.id, &name).await?;
        if entry.kind != EntryKind::File {
            return Err(ServiceError::Invalid(format!("'{name}' é uma pasta")));
        }
        // RN-106
        let dest = unique_file_path(local_dir, &name);
        provider
            .download(&entry.id, &dest)
            .await
            .map_err(CoreError::from)?;
        tracing::info!(account = %account_id, "arquivo baixado");
        Ok(dest)
    }
}

impl Service {
    /// Baixa um `.zip`/`.tar.*` da conta e o descompacta numa pasta nova dentro de
    /// `local_dir` (RN-108), com as proteções de RN-65 a RN-67. Progresso e cancelamento
    /// como na tela Compactar (uma operação dessas por vez).
    pub async fn download_and_extract(
        &self,
        account_id: Uuid,
        remote_path: &str,
        local_dir: PathBuf,
        progress: impl Fn(&ArchiveProgress) + Send + Sync + 'static,
    ) -> ServiceResult<ExtractResult> {
        let started = Instant::now();
        absolute_dir(&local_dir, "a pasta de destino")?;
        let path = RelPath::parse(remote_path).map_err(ServiceError::Invalid)?;
        let name = path
            .components()
            .last()
            .ok_or_else(|| ServiceError::Invalid("informe o caminho de um arquivo".into()))?
            .to_string();
        // Formato desconhecido falha antes de qualquer acesso à rede.
        let stem = archive_stem(&name)?;
        let format = ArchiveFormat::detect(&name).map_or(ArchiveFormat::Zip, |(f, _)| f);
        let parent = RelPath::parse(syncly_core::domain::rel_path::parent_str(path.as_str()))
            .map_err(ServiceError::Invalid)?;

        let provider = self.provider_by_id(account_id).await?;
        let folder = resolve_folder(provider.as_ref(), &parent, false).await?;
        let entry = single_child(provider.as_ref(), &folder.id, &name).await?;
        if entry.kind != EntryKind::File {
            return Err(ServiceError::Invalid(format!("'{name}' é uma pasta")));
        }

        let (_slot, cancel) = Slot::take(&self.archive_op)?;
        let staging = self.staging_dir(STAGING)?;
        ensure_space(&staging, entry.size.unwrap_or(0), "o disco do computador")?;
        let temp = TempFile(staging.join(format!(
            "{TMP_PREFIX}{}.{}",
            Uuid::new_v4(),
            format.extension()
        )));
        progress(&ArchiveProgress {
            bytes_total: entry.size.unwrap_or(0),
            current: Some(format!("baixando {name}")),
            ..ArchiveProgress::default()
        });
        // RN-106: o provedor confere o checksum do download.
        provider
            .download(&entry.id, &temp.0)
            .await
            .map_err(CoreError::from)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(CoreError::Cancelled.into());
        }
        let report =
            extract_new_folder(temp.0.clone(), &local_dir, &stem, cancel, progress).await?;
        tracing::info!(account = %account_id, "arquivo baixado e descompactado");
        Ok(ExtractResult::from_report(report, started))
    }

    /// Compacta pastas/arquivos e envia como um único arquivo para `remote_dir` (RN-109).
    /// Nunca sobrescreve nem duplica (RN-104); arquivos sensíveis ficam de fora (RN-105).
    pub async fn upload_compressed(
        &self,
        req: UploadCompressedRequest,
        progress: impl Fn(&ArchiveProgress) + Send + Sync + 'static,
    ) -> ServiceResult<RemoteItem> {
        let stem = check_inputs(&req.inputs, req.name.as_deref())?;
        let name = format!("{stem}.{}", req.format.extension());
        validate_name(&name).map_err(ServiceError::Invalid)?;
        let dir = RelPath::parse(&req.remote_dir).map_err(ServiceError::Invalid)?;
        let settings = self.load()?.settings;

        let provider = self.provider_by_id(req.account_id).await?;
        let folder = resolve_folder(provider.as_ref(), &dir, true).await?;
        // Antes de compactar, para não gastar tempo à toa.
        ensure_free_name(provider.as_ref(), &folder.id, &name, &dir).await?;

        let (_slot, cancel) = Slot::take(&self.archive_op)?;
        // RN-105: sensíveis nunca vão no envio avulso.
        let scan = self.scan_inputs(req.inputs.clone(), false).await?;
        let staging = self.staging_dir(STAGING)?;
        ensure_space(&staging, total_size(&scan), "o disco do computador")?;
        let temp = TempFile(staging.join(format!(
            "{TMP_PREFIX}{}.{}",
            Uuid::new_v4(),
            req.format.extension()
        )));

        let progress = Arc::new(progress);
        let (output, format, flag, report) =
            (temp.0.clone(), req.format, cancel.clone(), progress.clone());
        let created = blocking(move || {
            archive::create(&CreateRequest {
                scan: &scan,
                output: &output,
                format,
                level: None,
                cancel: &flag,
                progress: &*report,
            })
        })
        .await?;
        // RN-46: com o tamanho real, já compactado.
        if let Some(msg) = quota_shortfall(provider.as_ref(), &settings, created.bytes_out).await? {
            return Err(ServiceError::Invalid(msg));
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(CoreError::Cancelled.into());
        }
        progress(&ArchiveProgress {
            files_done: created.files,
            files_total: created.files,
            bytes_done: 0,
            bytes_total: created.bytes_out,
            current: Some(format!("enviando {name}")),
        });
        // Alguém pode ter criado o mesmo nome enquanto compactava.
        ensure_free_name(provider.as_ref(), &folder.id, &name, &dir).await?;
        let entry = provider
            .upload_new(&folder.id, &name, &temp.0)
            .await
            .map_err(CoreError::from)?;
        tracing::info!(account = %req.account_id, files = created.files, "pasta compactada enviada");
        Ok(to_item(dir.as_str(), entry))
    }
}

/// RN-104: nunca sobrescreve nem cria um segundo item com o mesmo nome.
async fn ensure_free_name(
    provider: &dyn StorageProvider,
    folder_id: &str,
    name: &str,
    dir: &RelPath,
) -> ServiceResult<()> {
    let taken = provider
        .child_names(folder_id)
        .await
        .map_err(CoreError::from)?;
    if taken.iter().any(|t| t == name) {
        return Err(ServiceError::Invalid(format!(
            "já existe '{name}' em Syncly/{dir} — nada foi sobrescrito"
        )));
    }
    Ok(())
}

/// Pasta dentro da raiz do app; com `create`, cria as que faltarem.
async fn resolve_folder(
    provider: &dyn StorageProvider,
    path: &RelPath,
    create: bool,
) -> ServiceResult<RemoteEntry> {
    resolve_path(provider, path, create)
        .await?
        .ok_or(ServiceError::NotFound("pasta remota"))
}

async fn single_child(
    provider: &dyn StorageProvider,
    folder_id: &str,
    name: &str,
) -> ServiceResult<RemoteEntry> {
    let found: Vec<_> = provider
        .list_children(folder_id)
        .await
        .map_err(CoreError::from)?
        .into_iter()
        .filter(|c| c.name == name)
        .collect();
    match found.as_slice() {
        [] => Err(ServiceError::NotFound("arquivo remoto")),
        [one] => Ok(one.clone()),
        _ => Err(duplicated(name)),
    }
}

/// RN-34: o app nunca "adivinha" entre itens com o mesmo nome.
fn duplicated(name: &str) -> ServiceError {
    ServiceError::Invalid(format!(
        "há mais de um item chamado '{name}' no destino — renomeie no provedor"
    ))
}

fn to_item(dir: &str, e: RemoteEntry) -> RemoteItem {
    RemoteItem {
        path: join_str(dir, &e.name),
        kind: match e.kind {
            EntryKind::File => "file",
            EntryKind::Folder => "folder",
        },
        downloadable: e.kind == EntryKind::File && e.hash.is_some(),
        size: e.size,
        modified_at: e.modified_at,
        modified_by: e.modified_by,
        name: e.name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::account_key;
    use crate::test_support::{GOOGLE_CLIENT_JSON, service};
    use serde_json::json;
    use syncly_core::domain::{Account, AccountKind};
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn rn104_rn105_rn106_transferencias_com_conta_local() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(&tmp.path().join("app"), "http://127.0.0.1:9");
        let hd = tmp.path().join("hd");
        std::fs::create_dir_all(&hd).unwrap();
        let account = svc.add_local_account("HD".into(), hd).unwrap();

        let local = tmp.path().join("a.txt");
        std::fs::write(&local, "conteúdo").unwrap();
        let item = svc
            .upload_file(account.id, &local, "Testes/Sub")
            .await
            .unwrap();
        assert_eq!(item.path, "Testes/Sub/a.txt");

        let listed = svc.list_remote(account.id, "Testes/Sub").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].downloadable);

        // RN-104: não sobrescreve nem duplica.
        let err = svc
            .upload_file(account.id, &local, "Testes/Sub")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("já existe"));

        // RN-105: arquivo sensível não sai do computador.
        let env = tmp.path().join(".env");
        std::fs::write(&env, "SECRET=1").unwrap();
        assert!(svc.upload_file(account.id, &env, "Testes").await.is_err());

        // RN-106: download nunca sobrescreve.
        let dl = tmp.path().join("downloads");
        std::fs::create_dir_all(&dl).unwrap();
        let first = svc
            .download_file(account.id, "Testes/Sub/a.txt", &dl)
            .await
            .unwrap();
        let second = svc
            .download_file(account.id, "Testes/Sub/a.txt", &dl)
            .await
            .unwrap();
        assert_eq!(first, dl.join("a.txt"));
        assert_eq!(second, dl.join("a (2).txt"));
        assert_eq!(std::fs::read_to_string(second).unwrap(), "conteúdo");
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        out.sort();
        out
    }

    #[tokio::test]
    async fn rn109_rn108_envia_pasta_compactada_e_baixa_descompactando() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(&tmp.path().join("app"), "http://127.0.0.1:9");
        let hd = tmp.path().join("hd");
        std::fs::create_dir_all(&hd).unwrap();
        let account = svc.add_local_account("HD".into(), hd.clone()).unwrap();
        let docs = tmp.path().join("Docs");
        std::fs::create_dir_all(docs.join("sub")).unwrap();
        std::fs::write(docs.join("a.txt"), "a").unwrap();
        std::fs::write(docs.join("sub/b.txt"), "b").unwrap();
        std::fs::write(docs.join(".env"), "SECRET=1").unwrap();

        let request = UploadCompressedRequest {
            account_id: account.id,
            inputs: vec![docs.clone()],
            remote_dir: "Testes".into(),
            format: ArchiveFormat::Zip,
            name: None,
        };
        let item = svc
            .upload_compressed(request.clone(), |_| {})
            .await
            .unwrap();
        assert_eq!(item.path, "Testes/Docs.zip");
        assert!(hd.join("Testes/Docs.zip").is_file());
        // RN-104: o mesmo nome de novo é recusado, nada é sobrescrito.
        let err = svc.upload_compressed(request, |_| {}).await.unwrap_err();
        assert!(err.to_string().contains("já existe"), "{err}");

        let out = tmp.path().join("baixados");
        std::fs::create_dir_all(&out).unwrap();
        let first = svc
            .download_and_extract(account.id, "Testes/Docs.zip", out.clone(), |_| {})
            .await
            .unwrap();
        let dest = PathBuf::from(&first.dest);
        assert_eq!(dest, out.join("Docs"));
        assert_eq!(
            std::fs::read_to_string(dest.join("sub/b.txt")).unwrap(),
            "b"
        );
        // RN-105: o arquivo sensível não saiu do computador.
        assert!(!dest.join(".env").exists());
        // RN-65: de novo, numa pasta nova.
        let second = svc
            .download_and_extract(account.id, "Testes/Docs.zip", out.clone(), |_| {})
            .await
            .unwrap();
        assert_eq!(PathBuf::from(second.dest), out.join("Docs (2)"));

        // Nenhum temporário sobra.
        let staging = svc.paths.data_dir.join("tmp").join(STAGING);
        assert!(names(&staging).is_empty());
    }

    #[tokio::test]
    async fn rn108_formato_desconhecido_e_recusado_antes_de_baixar() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(&tmp.path().join("app"), "http://127.0.0.1:9");
        let hd = tmp.path().join("hd");
        std::fs::create_dir_all(&hd).unwrap();
        let account = svc.add_local_account("HD".into(), hd).unwrap();
        let err = svc
            .download_and_extract(account.id, "Testes/foto.jpg", tmp.path().into(), |_| {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("formato"), "{err}");
    }

    fn add_google_account(svc: &Service) -> Uuid {
        std::fs::write(svc.paths.google_client_file(), GOOGLE_CLIENT_JSON).unwrap();
        let id = Uuid::new_v4();
        svc.mutate(|c| {
            c.accounts.push(Account {
                id,
                label: "eu@gmail.com".into(),
                kind: AccountKind::GoogleDrive {
                    email: Some("eu@gmail.com".into()),
                    root_folder_id: Some("ROOT1".into()),
                },
            });
            Ok(())
        })
        .unwrap();
        id
    }

    #[test]
    fn conta_inexistente_explica_como_conectar() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(&tmp.path().join("app"), "http://127.0.0.1:9");
        let hd = tmp.path().join("hd");
        std::fs::create_dir_all(&hd).unwrap();
        svc.add_local_account("HD".into(), hd).unwrap();
        let err = svc
            .find_account_id(Some("eu@gmail.com"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("contas: HD"), "{err}");
        assert!(err.contains("syncly google connect"), "{err}");
    }

    #[tokio::test]
    async fn conta_google_sem_credencial_pede_reconexao() {
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(&tmp.path().join("app"), "http://127.0.0.1:9");
        let id = add_google_account(&svc);
        let err = svc.list_remote(id, "").await.unwrap_err();
        assert!(err.to_string().contains("reconectada"), "{err}");
    }

    #[tokio::test]
    async fn conta_google_lista_pela_api_simulada() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "ya29.t", "expires_in": 3600
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/drive/v3/files/ROOT1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "ROOT1", "name": "Syncly", "mimeType": "application/vnd.google-apps.folder"
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/drive/v3/files"))
            .and(query_param("q", "'ROOT1' in parents and trashed = false"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "files": [
                { "id": "F1", "name": "a.txt", "mimeType": "text/plain", "size": "3",
                  "md5Checksum": "900150983cd24fb0d6963f7d28e17f72",
                  "lastModifyingUser": { "displayName": "Ana Teste" } },
                { "id": "D1", "name": "Docs", "mimeType": "application/vnd.google-apps.folder" },
                { "id": "G1", "name": "Planilha", "mimeType": "application/vnd.google-apps.spreadsheet" }
            ]})))
            .mount(&server)
            .await;

        let tmp = tempfile::tempdir().unwrap();
        let svc = service(&tmp.path().join("app"), &server.uri());
        let id = add_google_account(&svc);
        svc.secrets.set(&account_key(id), "1//r").unwrap();

        let items = svc.list_remote(id, "").await.unwrap();
        let names: Vec<_> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["Docs", "a.txt", "Planilha"]);
        assert_eq!(items[1].modified_by.as_deref(), Some("Ana Teste"));
        // RN-34: documento nativo do Google não tem conteúdo para baixar.
        assert!(!items[2].downloadable);
    }

    #[tokio::test]
    async fn seg06_remover_conta_google_revoga_e_apaga_credencial() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/revoke"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let tmp = tempfile::tempdir().unwrap();
        let svc = service(&tmp.path().join("app"), &server.uri());
        let id = add_google_account(&svc);
        svc.secrets.set(&account_key(id), "1//r").unwrap();

        svc.remove_account(id).await.unwrap();
        assert_eq!(svc.secrets.get(&account_key(id)).unwrap(), None);
        assert!(svc.list_accounts().unwrap().is_empty());
    }
}
