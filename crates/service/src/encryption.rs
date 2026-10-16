//! Chaves da criptografia das versões (RN-71 a RN-74, SEG-61). A pública fica no
//! `config.toml`; a privada só no keyring e no arquivo de recuperação que o usuário guarda —
//! nunca passa pela interface (SEG-04).

use std::io::{Read, Write};
use std::path::Path;

use chrono::Utc;
use syncly_core::CoreError;
use syncly_core::config::AppConfig;
use syncly_core::crypto::{self, PublicKey, SecretKey};
use syncly_core::fsutil::create_private_file;

use crate::dto::{EncryptionStatus, NewEncryptionKey};
use crate::error::{ServiceError, ServiceResult};
use crate::secrets::encryption_key;
use crate::service::Service;

fn status(config: &AppConfig) -> EncryptionStatus {
    EncryptionStatus {
        active: config.encryption.active().map(|k| k.fingerprint()),
        known: config
            .encryption
            .known()
            .iter()
            .map(PublicKey::fingerprint)
            .collect(),
    }
}

fn io_error(path: &Path, source: std::io::Error) -> ServiceError {
    CoreError::Io {
        path: path.to_path_buf(),
        source,
    }
    .into()
}

impl Service {
    pub fn encryption_status(&self) -> ServiceResult<EncryptionStatus> {
        Ok(status(&self.load()?))
    }

    /// RN-72, passo 1: gera um par de chaves, guarda a privada no keyring e grava o arquivo
    /// de recuperação `syncly-chave-<impressão digital>.txt` em `folder` (permissão `0600`,
    /// nunca por cima de outro arquivo). A chave ainda **não** vale para novas versões.
    pub async fn create_encryption_key(&self, folder: &Path) -> ServiceResult<NewEncryptionKey> {
        if !folder.is_absolute() || !folder.is_dir() {
            return Err(ServiceError::Invalid(format!(
                "'{}' não é uma pasta existente",
                folder.display()
            )));
        }
        let key = SecretKey::generate();
        let public = key.public();
        let fingerprint = public.fingerprint();
        let file = folder.join(format!("syncly-chave-{fingerprint}.txt"));
        if file.exists() {
            return Err(ServiceError::Invalid(format!(
                "já existe '{}' — nada foi sobrescrito",
                file.display()
            )));
        }
        // Keyring primeiro: se falhar, nenhum arquivo de chave fica solto no disco.
        self.secret_set(encryption_key(&fingerprint), key.expose())
            .await?;
        let content = crypto::recovery_file(&key, Utc::now());
        let mut out = create_private_file(&file).map_err(|e| io_error(&file, e))?;
        out.write_all(content.as_bytes())
            .and_then(|()| out.sync_all())
            .map_err(|e| io_error(&file, e))?;
        tracing::info!(key = %fingerprint, "chave de criptografia criada");
        Ok(NewEncryptionKey {
            fingerprint,
            public_key: public.to_string(),
            recovery_file: file,
        })
    }

    /// RN-72, passo 2: com a confirmação de que o arquivo de recuperação foi guardado, a
    /// chave passa a cifrar as novas versões. A anterior continua conhecida (RN-74).
    pub async fn activate_encryption_key(
        &self,
        public_key: &str,
        saved: bool,
    ) -> ServiceResult<EncryptionStatus> {
        if !saved {
            return Err(ServiceError::Invalid(
                "confirme que guardou a chave de recuperação — sem ela, as versões \
                 criptografadas não podem ser abertas"
                    .into(),
            ));
        }
        let public = PublicKey::parse(public_key).map_err(ServiceError::Invalid)?;
        // A privada correspondente precisa estar neste computador.
        let stored = self
            .secret_get(encryption_key(&public.fingerprint()))
            .await?
            .and_then(|s| SecretKey::parse(&s).ok());
        if stored.is_none_or(|k| k.public().fingerprint() != public.fingerprint()) {
            return Err(ServiceError::Invalid(
                "esta chave não está no cofre de senhas deste computador — crie ou importe a \
                 chave de novo"
                    .into(),
            ));
        }
        self.use_key(&public)
    }

    /// Importa um arquivo de recuperação (outro computador, reinstalação ou chave antiga,
    /// RN-74). Se ainda não houver chave ativa, esta passa a ser a ativa — quem tem o
    /// arquivo já guardou a recuperação (RN-72).
    pub async fn import_encryption_key(&self, file: &Path) -> ServiceResult<EncryptionStatus> {
        let text = read_key_file(file)?;
        let key = crypto::parse_recovery_file(&text).map_err(ServiceError::Invalid)?;
        let public = key.public();
        self.secret_set(encryption_key(&public.fingerprint()), key.expose())
            .await?;
        tracing::info!(key = %public.fingerprint(), "chave de criptografia importada");
        let make_active = self.load()?.encryption.active().is_none();
        let public_text = public.to_string();
        self.mutate(move |c| {
            if !c.encryption.known_keys.contains(&public_text) {
                c.encryption.known_keys.push(public_text.clone());
            }
            if make_active {
                c.encryption.public_key = Some(public_text);
            }
            Ok(status(c))
        })
    }

    fn use_key(&self, public: &PublicKey) -> ServiceResult<EncryptionStatus> {
        let text = public.to_string();
        let result = self.mutate(move |c| {
            if !c.encryption.known_keys.contains(&text) {
                c.encryption.known_keys.push(text.clone());
            }
            c.encryption.public_key = Some(text);
            Ok(status(c))
        })?;
        tracing::info!(key = %public.fingerprint(), "chave de criptografia ativada");
        Ok(result)
    }

    /// RN-73: para cifrar basta a chave pública — nada do keyring é lido.
    pub(crate) fn encryption_public_key(&self) -> ServiceResult<PublicKey> {
        self.load()?.encryption.active().ok_or_else(|| {
            ServiceError::Invalid(
                "ative a criptografia em Preferências → Criptografia antes de criptografar".into(),
            )
        })
    }

    /// RN-73, RN-74: as chaves privadas só são lidas para abrir uma versão criptografada —
    /// todas as conhecidas, já que a versão pode ser de uma chave antiga.
    pub(crate) async fn decryption_keys(&self) -> ServiceResult<Vec<SecretKey>> {
        let mut keys = Vec::new();
        for public in self.load()?.encryption.known() {
            let fingerprint = public.fingerprint();
            if let Some(key) = self
                .secret_get(encryption_key(&fingerprint))
                .await?
                .and_then(|s| SecretKey::parse(&s).ok())
                .filter(|k| k.public().fingerprint() == fingerprint)
            {
                keys.push(key);
            }
        }
        Ok(keys)
    }

    /// Chaves para abrir `file_name`: nenhuma se não for `.age` (o keyring nem é lido).
    pub(crate) async fn keys_for(&self, file_name: &str) -> ServiceResult<Vec<SecretKey>> {
        if crypto::strip_extension(file_name).is_some() {
            self.decryption_keys().await
        } else {
            Ok(Vec::new())
        }
    }
}

/// Lê um arquivo de chave pequeno; qualquer coisa maior não é uma chave.
fn read_key_file(file: &Path) -> ServiceResult<String> {
    let meta = std::fs::metadata(file).map_err(|e| io_error(file, e))?;
    if !meta.is_file() || meta.len() > crypto::MAX_KEY_FILE_BYTES {
        return Err(ServiceError::Invalid(format!(
            "'{}' não é um arquivo de chave",
            file.display()
        )));
    }
    let mut text = String::new();
    std::fs::File::open(file)
        .and_then(|mut f| f.read_to_string(&mut text))
        .map_err(|e| io_error(file, e))?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use crate::archive::CompressRequest;
    use crate::dto::{JobDraft, SyncDraft};
    use crate::service::RunOptions;
    use crate::test_support::{FakeOs, RecordingStore, service_with_secrets};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use syncly_core::archive::ArchiveFormat;
    use syncly_core::crypto::{self, MAGIC};
    use syncly_core::domain::{Delivery, Retention, RunTrigger, SyncMode};
    use uuid::Uuid;

    struct Setup {
        tmp: tempfile::TempDir,
        store: Arc<RecordingStore>,
        service: crate::Service,
        source: PathBuf,
        backups: PathBuf,
        keys: PathBuf,
        account: Uuid,
    }

    fn setup() -> Setup {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(RecordingStore::default());
        let service = service_with_secrets(
            &tmp.path().join("app"),
            "http://127.0.0.1:9",
            store.clone(),
            Arc::new(FakeOs::default()),
        );
        let source = tmp.path().join("Mundo");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("level.dat"), "segredo do mundo").unwrap();
        let hd = tmp.path().join("hd");
        let keys = tmp.path().join("pendrive");
        std::fs::create_dir_all(&hd).unwrap();
        std::fs::create_dir_all(&keys).unwrap();
        let account = service
            .add_local_account("HD".into(), hd.clone())
            .unwrap()
            .id;
        Setup {
            backups: hd.join("Backups"),
            tmp,
            store,
            service,
            source,
            keys,
            account,
        }
    }

    /// Cria a chave e confirma que o arquivo de recuperação foi guardado (RN-72).
    async fn active_key(s: &Setup) -> String {
        let key = s.service.create_encryption_key(&s.keys).await.unwrap();
        s.service
            .activate_encryption_key(&key.public_key, true)
            .await
            .unwrap();
        key.fingerprint
    }

    fn encrypted_job(s: &Setup) -> Uuid {
        s.service
            .save_job(JobDraft {
                id: None,
                name: "Mundo".into(),
                enabled: true,
                sources: vec![s.source.clone()],
                exclude: vec![],
                include_sensitive: false,
                sync: Some(SyncDraft {
                    account_id: s.account,
                    path: "Backups".into(),
                    delivery: Delivery::Archive {
                        format: ArchiveFormat::TarZst,
                        retention: Retention::default(),
                        encrypt: true,
                    },
                    mode: SyncMode::Upload,
                    schedule: None,
                }),
            })
            .unwrap()
            .id
    }

    fn manual() -> RunOptions {
        RunOptions {
            confirm: true,
            trigger: RunTrigger::Manual,
        }
    }

    fn only_file(dir: &Path) -> PathBuf {
        let files: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(files.len(), 1, "{files:?}");
        files[0].clone()
    }

    #[tokio::test]
    async fn rn72_chave_so_vale_depois_de_confirmar_o_arquivo_de_recuperacao() {
        let s = setup();
        let key = s.service.create_encryption_key(&s.keys).await.unwrap();
        // O arquivo de recuperação é privado e devolve a mesma chave do keyring.
        assert_eq!(
            key.recovery_file,
            s.keys.join(format!("syncly-chave-{}.txt", key.fingerprint))
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&key.recovery_file)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let text = std::fs::read_to_string(&key.recovery_file).unwrap();
        let recovered = crypto::parse_recovery_file(&text).unwrap();
        assert_eq!(recovered.public().to_string(), key.public_key);

        // Ainda não ativa: nenhuma tarefa pode cifrar.
        assert_eq!(s.service.encryption_status().unwrap().active, None);
        let draft_err = s
            .service
            .save_job(JobDraft {
                id: None,
                name: "x".into(),
                enabled: true,
                sources: vec![s.source.clone()],
                exclude: vec![],
                include_sensitive: false,
                sync: Some(SyncDraft {
                    account_id: s.account,
                    path: "X".into(),
                    delivery: Delivery::Archive {
                        format: ArchiveFormat::TarZst,
                        retention: Retention::default(),
                        encrypt: true,
                    },
                    mode: SyncMode::Upload,
                    schedule: None,
                }),
            })
            .unwrap_err();
        assert!(
            draft_err.to_string().contains("ative a criptografia"),
            "{draft_err}"
        );

        // Sem a confirmação, não ativa.
        let err = s
            .service
            .activate_encryption_key(&key.public_key, false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("guardou"), "{err}");
        // Chave que não está no keyring deste computador, também não.
        let stranger = crypto::SecretKey::generate().public().to_string();
        assert!(
            s.service
                .activate_encryption_key(&stranger, true)
                .await
                .is_err()
        );

        let status = s
            .service
            .activate_encryption_key(&key.public_key, true)
            .await
            .unwrap();
        assert_eq!(status.active.as_deref(), Some(key.fingerprint.as_str()));
        // Uma chave, um arquivo de recuperação.
        assert_eq!(std::fs::read_dir(&s.keys).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn rn70_rn73_rn74_versoes_criptografadas_e_troca_de_chave() {
        let s = setup();
        let first = active_key(&s).await;
        let job = encrypted_job(&s);
        let reads_before = s.store.reads().len();

        let report = s.service.run_job(job, manual(), |_| {}).await.unwrap();
        assert_eq!(report.status, "success");
        // RN-73: cifrar não lê nenhuma chave privada.
        assert_eq!(s.store.reads().len(), reads_before);
        let version = only_file(&s.backups);
        let name = version.file_name().unwrap().to_string_lossy().to_string();
        assert!(name.ends_with(".tar.zst.age"), "{name}");
        let bytes = std::fs::read(&version).unwrap();
        assert!(bytes.starts_with(MAGIC));
        assert!(!bytes.windows(7).any(|w| w == b"segredo"));
        let versions = s.service.list_versions(job).unwrap();
        assert_eq!(versions[0].key_fingerprint.as_deref(), Some(first.as_str()));

        // RN-74: chave nova para as próximas versões; a antiga continua abrindo.
        let second = active_key(&s).await;
        assert_ne!(first, second);
        std::fs::write(s.source.join("level.dat"), "mundo mudou").unwrap();
        s.service.run_job(job, manual(), |_| {}).await.unwrap();
        let versions = s.service.list_versions(job).unwrap();
        assert_eq!(
            versions[0].key_fingerprint.as_deref(),
            Some(second.as_str())
        );
        assert_eq!(versions[1].key_fingerprint.as_deref(), Some(first.as_str()));

        let out = s.tmp.path().join("restaurado");
        std::fs::create_dir_all(&out).unwrap();
        let old = s
            .service
            .restore_version(job, versions[1].id, &out, |_| {})
            .await
            .unwrap();
        let restored = PathBuf::from(&old.dest);
        assert_eq!(
            std::fs::read_to_string(restored.join("level.dat")).unwrap(),
            "segredo do mundo"
        );
        // Restaurar, sim, lê as chaves privadas.
        assert!(s.store.reads().iter().any(|k| k.starts_with("age-key:")));
    }

    #[tokio::test]
    async fn outro_computador_abre_so_depois_de_importar_a_chave() {
        let s = setup();
        let fingerprint = active_key(&s).await;
        let job = encrypted_job(&s);
        s.service.run_job(job, manual(), |_| {}).await.unwrap();
        let version = only_file(&s.backups);

        // "Outro computador": outro app, sem a chave.
        let other = setup();
        let dest = other.tmp.path().join("recebido");
        std::fs::create_dir_all(&dest).unwrap();
        let err = other
            .service
            .extract(version.clone(), dest.clone(), |_| {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("importe a chave"), "{err}");
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0);

        let recovery = s.keys.join(format!("syncly-chave-{fingerprint}.txt"));
        let status = other
            .service
            .import_encryption_key(&recovery)
            .await
            .unwrap();
        // Importar num app sem chave já a deixa ativa (quem tem o arquivo já o guardou).
        assert_eq!(status.active.as_deref(), Some(fingerprint.as_str()));
        let result = other.service.extract(version, dest, |_| {}).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(PathBuf::from(result.dest).join("level.dat")).unwrap(),
            "segredo do mundo"
        );
        // Arquivo que não é chave é recusado.
        let junk = other.tmp.path().join("nao-e-chave.txt");
        std::fs::write(&junk, "oi").unwrap();
        assert!(other.service.import_encryption_key(&junk).await.is_err());
    }

    #[tokio::test]
    async fn rn63_rn70_compactar_com_criptografia() {
        let s = setup();
        let out = s.tmp.path().join("saida");
        std::fs::create_dir_all(&out).unwrap();
        let request = |format, encrypt| CompressRequest {
            inputs: vec![s.source.clone()],
            output_dir: out.clone(),
            name: None,
            format,
            include_sensitive: false,
            encrypt,
        };
        // Sem chave ativa, ou fora do TAR.ZST: recusado antes de compactar.
        let err = s
            .service
            .compress(request(ArchiveFormat::TarZst, true), |_| {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("ative a criptografia"), "{err}");
        active_key(&s).await;
        assert!(
            s.service
                .compress(request(ArchiveFormat::Zip, true), |_| {})
                .await
                .is_err()
        );
        let done = s
            .service
            .compress(request(ArchiveFormat::TarZst, true), |_| {})
            .await
            .unwrap();
        assert!(done.path.ends_with("Mundo.tar.zst.age"), "{}", done.path);
        assert!(std::fs::read(&done.path).unwrap().starts_with(MAGIC));
    }
}
