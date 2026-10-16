//! `config.toml`: o que o usuário quer (contas, tarefas, preferências).
//! Legível, versionado e gravado atomicamente com permissão `0600`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::{Account, AccountKind, Job, ValidationContext};
use crate::error::{CoreError, CoreResult, IoContext, ValidationIssue};
use crate::fsutil::write_atomic;

pub const CONFIG_VERSION: u32 = 1;
/// RN-46: maior reserva de espaço aceita (1 TiB).
pub const MAX_FREE_SPACE_MIB: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyPolicy {
    /// Também avisa quando dá certo.
    Always,
    /// RN-94: só o que precisa de atenção.
    #[default]
    Failures,
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// SEG-41: máximo 8.
    pub max_parallel_transfers: u8,
    /// SEG-41: limitador de taxa por processo.
    pub max_requests_per_second: u8,
    /// RN-46: reserva mínima de espaço na conta.
    pub min_free_space_mib: u64,
    pub notify: NotifyPolicy,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_parallel_transfers: 4,
            max_requests_per_second: 8,
            min_free_space_mib: 1024,
            notify: NotifyPolicy::Failures,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub version: u32,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default, rename = "account", skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<Account>,
    #[serde(default, rename = "job", skip_serializing_if = "Vec::is_empty")]
    pub jobs: Vec<Job>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            settings: Settings::default(),
            accounts: Vec::new(),
            jobs: Vec::new(),
        }
    }
}

impl AppConfig {
    /// Arquivo ausente = configuração vazia.
    pub fn load(path: &Path) -> CoreResult<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e).at(path),
        };
        let config: Self = toml::from_str(&text)
            .map_err(|e| CoreError::Config(format!("{}: {e}", path.display())))?;
        if config.version > CONFIG_VERSION {
            return Err(CoreError::Config(format!(
                "config versão {} é mais nova que este app (suporta até {CONFIG_VERSION})",
                config.version
            )));
        }
        Ok(config)
    }

    /// Valida e grava (SEG-14, SEG-53). Nada inválido chega ao disco.
    pub fn save(&self, path: &Path, app_dirs: &[PathBuf]) -> CoreResult<()> {
        let issues = self.validate(app_dirs);
        if !issues.is_empty() {
            return Err(CoreError::Invalid(issues));
        }
        let text = toml::to_string_pretty(self)
            .map_err(|e| CoreError::Config(format!("falha ao serializar: {e}")))?;
        let header = "# Configuração do Syncly — veja docs/REGRAS_DE_NEGOCIO.md\n\n";
        write_atomic(path, format!("{header}{text}").as_bytes())
    }

    pub fn account(&self, id: Uuid) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }

    pub fn job(&self, id: Uuid) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }

    /// Busca por ID ou por nome (sem diferenciar maiúsculas) — usado pela CLI.
    pub fn find_job(&self, key: &str) -> Option<&Job> {
        if let Ok(id) = key.parse::<Uuid>() {
            return self.job(id);
        }
        self.jobs
            .iter()
            .find(|j| j.name.trim().eq_ignore_ascii_case(key.trim()))
    }

    /// RN-19: destino local não pode estar dentro de uma fonte, nem conter uma — a cópia
    /// passaria a copiar a si mesma a cada execução.
    pub fn validate_local_destination(&self, job: &Job) -> Vec<ValidationIssue> {
        let Some(sync) = &job.sync else {
            return Vec::new();
        };
        let Some(AccountKind::Local { root }) =
            self.account(sync.destination.account_id).map(|a| &a.kind)
        else {
            return Vec::new();
        };
        let dest = sync
            .destination
            .path
            .components()
            .fold(root.clone(), |acc, part| acc.join(part));
        job.sources
            .iter()
            .filter(|source| dest.starts_with(source) || source.starts_with(&dest))
            .map(|source| {
                ValidationIssue::new(
                    format!("tarefa '{}'", job.name.trim()),
                    format!(
                        "o destino '{}' e a fonte '{}' estão um dentro do outro",
                        dest.display(),
                        source.display()
                    ),
                )
            })
            .collect()
    }

    pub fn validate(&self, app_dirs: &[PathBuf]) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        let s = &self.settings;
        if !(1..=8).contains(&s.max_parallel_transfers) {
            issues.push(ValidationIssue::new(
                "preferências.transferências paralelas",
                "deve estar entre 1 e 8",
            ));
        }
        // RN-46: até 1 TiB de reserva.
        if s.min_free_space_mib > MAX_FREE_SPACE_MIB {
            issues.push(ValidationIssue::new(
                "preferências.reserva de espaço",
                "deve ser de no máximo 1 TiB",
            ));
        }
        if !(1..=10).contains(&s.max_requests_per_second) {
            issues.push(ValidationIssue::new(
                "preferências.requisições por segundo",
                "deve estar entre 1 e 10",
            ));
        }

        let mut ids = HashSet::new();
        for account in &self.accounts {
            if !ids.insert(account.id) {
                issues.push(ValidationIssue::new("contas", "ID de conta repetido"));
            }
            issues.extend(account.validate());
        }

        let ctx = ValidationContext { app_dirs };
        let mut job_ids = HashSet::new();
        let mut names = HashSet::new();
        for job in &self.jobs {
            let label = format!("tarefa '{}'", job.name.trim());
            if !job_ids.insert(job.id) {
                issues.push(ValidationIssue::new(&label, "ID repetido"));
            }
            if !names.insert(job.name.trim().to_lowercase()) {
                issues.push(ValidationIssue::new(
                    &label,
                    "já existe uma tarefa com esse nome",
                ));
            }
            issues.extend(job.validate(&ctx));

            // RN-05: tarefa desativada pode apontar para conta removida.
            if let Some(sync) = &job.sync
                && job.enabled
                && self.account(sync.destination.account_id).is_none()
            {
                issues.push(ValidationIssue::new(&label, "conta de destino não existe"));
            }
        }

        for job in &self.jobs {
            issues.extend(self.validate_local_destination(job));
        }

        // RN-17: cada pasta remota tem um único dono. Quem só recebe (RN-110) não é dono.
        let syncs: Vec<_> = self
            .jobs
            .iter()
            .filter_map(|j| j.sync.as_ref().map(|s| (j, s)))
            .filter(|(_, s)| !s.delivery.is_receive())
            .map(|(j, s)| (j, &s.destination))
            .collect();
        for (i, (a, da)) in syncs.iter().enumerate() {
            for (b, db) in &syncs[i + 1..] {
                if da.account_id == db.account_id
                    && (da.path.is_within(&db.path) || db.path.is_within(&da.path))
                {
                    issues.push(ValidationIssue::new(
                        format!("tarefa '{}'", a.name.trim()),
                        format!(
                            "destino '{}' se sobrepõe ao da tarefa '{}'",
                            da.path,
                            b.name.trim()
                        ),
                    ));
                }
            }
        }
        issues
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        AccountKind, Delivery, Destination, GuardLimits, RelPath, Schedule, SyncMode, SyncSettings,
    };

    fn sample() -> AppConfig {
        let account = Account {
            id: Uuid::new_v4(),
            label: "HD externo".into(),
            kind: AccountKind::Local {
                root: "/mnt/hd".into(),
            },
        };
        let job = Job {
            id: Uuid::new_v4(),
            name: "Documentos".into(),
            enabled: true,
            sources: vec!["/home/u/Documentos".into()],
            exclude: vec!["*.log".into()],
            include_sensitive: false,
            sync: Some(SyncSettings {
                destination: Destination {
                    account_id: account.id,
                    path: RelPath::parse("Docs").unwrap(),
                },
                delivery: Delivery::Files,
                mode: SyncMode::Mirror,
                schedule: Schedule::Daily {
                    at: "02:00".to_string().try_into().unwrap(),
                },
                guard: GuardLimits::default(),
            }),
        };
        AppConfig {
            accounts: vec![account],
            jobs: vec![job],
            ..AppConfig::default()
        }
    }

    #[test]
    fn grava_e_le_de_volta() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let config = sample();
        config.save(&path, &[]).unwrap();
        assert_eq!(AppConfig::load(&path).unwrap(), config);
    }

    #[test]
    fn arquivo_ausente_e_config_vazia() {
        let dir = tempfile::tempdir().unwrap();
        let config = AppConfig::load(&dir.path().join("nao-existe.toml")).unwrap();
        assert!(config.jobs.is_empty());
    }

    #[test]
    fn recusa_versao_futura() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "version = 99\n").unwrap();
        assert!(AppConfig::load(&path).is_err());
    }

    #[test]
    fn nao_grava_config_invalida() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = sample();
        config.accounts.clear(); // tarefa aponta para conta inexistente
        assert!(matches!(
            config.save(&path, &[]),
            Err(CoreError::Invalid(_))
        ));
        assert!(!path.exists());
    }

    #[test]
    fn rn19_destino_local_dentro_da_fonte() {
        let mut config = sample();
        // Conta local em /mnt/hd; destino "Docs" → /mnt/hd/Docs.
        config.jobs[0].sources = vec!["/mnt/hd".into()];
        assert!(
            config
                .validate(&[])
                .iter()
                .any(|i| i.message.contains("um dentro do outro"))
        );
        config.jobs[0].sources = vec!["/mnt/hd/Docs/sub".into()];
        assert!(!config.validate(&[]).is_empty());
        config.jobs[0].sources = vec!["/home/u/Documentos".into()];
        assert!(config.validate(&[]).is_empty());
    }

    #[test]
    fn rn17_destinos_sobrepostos() {
        let mut config = sample();
        let mut other = config.jobs[0].clone();
        other.id = Uuid::new_v4();
        other.name = "Outra".into();
        other.sources = vec!["/home/u/Outra".into()];
        if let Some(s) = other.sync.as_mut() {
            s.destination.path = RelPath::parse("Docs/Sub").unwrap();
        }
        config.jobs.push(other);
        assert!(
            config
                .validate(&[])
                .iter()
                .any(|i| i.message.contains("sobrepõe"))
        );
    }
}
