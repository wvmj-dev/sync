//! Tarefa ("rotina"): fontes + destino + forma de entrega (RN-10 a RN-19, RN-50 a RN-55,
//! RN-107, RN-110).

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::archive_format::ArchiveFormat;
use super::check_label;
use super::rel_path::{RelPath, validate_name};
use super::run::RunKind;
use super::schedule::Schedule;
use crate::error::ValidationIssue;
use crate::filters::Filter;

pub const MAX_SOURCES: usize = 32;
const SYSTEM_DIRS: [&str; 4] = ["/proc", "/sys", "/dev", "/run"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    /// Local → nuvem, sem propagar exclusões. Padrão e mais seguro.
    #[default]
    Upload,
    /// Local → nuvem, exclusões locais vão para a lixeira (sob guardas).
    Mirror,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Destination {
    pub account_id: Uuid,
    /// Relativo à pasta raiz do app na conta (RN-03).
    pub path: RelPath,
}

/// Limites da guarda contra exclusão em massa (RN-42).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuardLimits {
    pub max_trash_files: u32,
    pub max_trash_percent: u8,
}

impl Default for GuardLimits {
    fn default() -> Self {
        Self {
            max_trash_files: 50,
            max_trash_percent: 25,
        }
    }
}

/// O que a tarefa faz — a "forma" da rotina.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Delivery {
    /// Envia arquivo por arquivo: sincronização (modos Envio/Espelho, RN-30 a RN-39).
    #[default]
    Files,
    /// Envia um arquivo compactado por execução, com versões (RN-50 a RN-55, RN-59).
    Archive {
        format: ArchiveFormat,
        #[serde(default)]
        retention: Retention,
        /// RN-70: cifra cada versão com a chave pública do app (`.tar.zst.age`, RN-63).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        encrypt: bool,
    },
    /// Recebe: baixa a versão mais recente do destino e a descompacta numa pasta nova
    /// dentro de `target` (RN-107). Não tem fontes (RN-110).
    Receive { target: PathBuf },
}

impl Delivery {
    pub fn is_receive(&self) -> bool {
        matches!(self, Self::Receive { .. })
    }

    /// Tipo de execução que a tarefa registra no histórico (e que conta para RN-36).
    pub fn run_kind(&self) -> RunKind {
        match self {
            Self::Files => RunKind::Sync,
            Self::Archive { .. } => RunKind::Backup,
            Self::Receive { .. } => RunKind::Receive,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncSettings {
    pub destination: Destination,
    #[serde(default)]
    pub delivery: Delivery,
    /// Só vale para [`Delivery::Files`].
    #[serde(default)]
    pub mode: SyncMode,
    #[serde(default)]
    pub schedule: Schedule,
    #[serde(default)]
    pub guard: GuardLimits,
}

/// Retenção estilo GFS (RN-54).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Retention {
    pub keep_last: u16,
    pub daily: u16,
    pub weekly: u16,
    pub monthly: u16,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            keep_last: 3,
            daily: 7,
            weekly: 4,
            monthly: 6,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: Uuid,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub sources: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    /// RN-22: arquivos sensíveis só entram se marcado explicitamente.
    #[serde(default)]
    pub include_sensitive: bool,
    /// Destino e forma de entrega (RN-15: obrigatório).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<SyncSettings>,
}

/// Informações externas necessárias para validar uma tarefa.
pub struct ValidationContext<'a> {
    /// Pastas do próprio app (RN-12).
    pub app_dirs: &'a [PathBuf],
}

/// RN-12: fora das pastas de sistema e das pastas do próprio app.
fn check_local_dir(path: &Path, ctx: &ValidationContext<'_>) -> Result<(), String> {
    let shown = path.display();
    if !path.is_absolute() {
        return Err(format!("'{shown}' deve ser um caminho absoluto"));
    }
    if path
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(format!("'{shown}' não pode conter '.' ou '..'"));
    }
    if path == Path::new("/") || SYSTEM_DIRS.iter().any(|d| path.starts_with(d)) {
        return Err(format!("'{shown}' é uma pasta de sistema"));
    }
    if ctx.app_dirs.iter().any(|d| path.starts_with(d)) {
        return Err(format!("'{shown}' fica dentro das pastas do Syncly"));
    }
    Ok(())
}

impl Job {
    /// Nome de cada fonte no destino (RN-14). `None` para caminhos sem nome (ex.: `/`).
    pub fn source_name(source: &Path) -> Option<&str> {
        source.file_name().and_then(|n| n.to_str())
    }

    pub fn validate(&self, ctx: &ValidationContext<'_>) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        let mut push = |field: &str, msg: String| {
            issues.push(ValidationIssue::new(
                format!("tarefa '{}'.{field}", self.name.trim()),
                msg,
            ));
        };

        // RN-10
        if let Err(e) = check_label(&self.name, 64) {
            push("nome", e);
        }

        let receive_target = match self.sync.as_ref().map(|s| &s.delivery) {
            Some(Delivery::Receive { target }) => Some(target),
            _ => None,
        };
        // RN-11 a RN-14; RN-110: quem recebe não tem fontes.
        match receive_target {
            Some(_) if !self.sources.is_empty() => {
                push("fontes", "uma tarefa que recebe não tem fontes".into());
            }
            Some(_) => {}
            None if self.sources.is_empty() || self.sources.len() > MAX_SOURCES => {
                push("fontes", format!("informe de 1 a {MAX_SOURCES} fontes"));
            }
            None => {}
        }
        // RN-110: a pasta que recebe segue as mesmas restrições de uma fonte (RN-12).
        if let Some(target) = receive_target
            && let Err(e) = check_local_dir(target, ctx)
        {
            push("pasta no computador", e);
        }
        let mut names = HashSet::new();
        for (i, src) in self.sources.iter().enumerate() {
            let shown = src.display();
            if let Err(e) = check_local_dir(src, ctx) {
                push("fontes", e);
                continue;
            }
            match Self::source_name(src) {
                Some(name) if validate_name(name).is_ok() => {
                    if !names.insert(name.to_lowercase()) {
                        push(
                            "fontes",
                            format!("duas fontes com o mesmo nome '{name}' (RN-14)"),
                        );
                    }
                }
                _ => push("fontes", format!("'{shown}' não tem um nome válido")),
            }
            for other in &self.sources[i + 1..] {
                if src == other {
                    push("fontes", format!("'{shown}' está repetida"));
                } else if src.starts_with(other) || other.starts_with(src) {
                    push(
                        "fontes",
                        format!(
                            "'{shown}' e '{}' estão uma dentro da outra",
                            other.display()
                        ),
                    );
                }
            }
        }

        // RN-20 e RN-22: a própria fonte não pode casar com uma exclusão (ex.: `~/.ssh`).
        match Filter::new(&self.exclude, self.include_sensitive) {
            Err(e) => push("exclusões", e),
            Ok(filter) => {
                for src in &self.sources {
                    let Some(name) = Self::source_name(src) else {
                        continue;
                    };
                    let name = Path::new(name);
                    if filter.is_excluded(name, true) || filter.is_excluded(name, false) {
                        push(
                            "fontes",
                            format!(
                                "'{}' é excluída pelos filtros (arquivos sensíveis exigem \"incluir sensíveis\")",
                                src.display()
                            ),
                        );
                    }
                }
            }
        }

        // RN-15
        if self.sync.is_none() {
            push("destino", "escolha a conta e a pasta de destino".into());
        }

        if let Some(sync) = &self.sync {
            // RN-16
            if sync.destination.path.is_root() {
                push("destino", "informe uma pasta de destino".into());
            }
            // RN-54: pelo menos a versão mais recente é mantida.
            if let Delivery::Archive { retention, .. } = sync.delivery
                && retention.keep_last < 1
            {
                push("versões", "mantenha pelo menos a última versão".into());
            }
            // RN-63: só o TAR.ZST é criptografado (o ZIP precisa de acesso aleatório).
            if let Delivery::Archive {
                format,
                encrypt: true,
                ..
            } = sync.delivery
                && format != ArchiveFormat::TarZst
            {
                push(
                    "criptografia",
                    "a criptografia exige o formato TAR.ZST".into(),
                );
            }
            if !(1..=100).contains(&sync.guard.max_trash_percent) {
                push("proteção", "percentual deve estar entre 1 e 100".into());
            }
            if let Err(e) = sync.schedule.validate() {
                push("agendamento", e);
            }
        }

        issues
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(sources: &[&str]) -> Job {
        Job {
            id: Uuid::new_v4(),
            name: "Documentos".into(),
            enabled: true,
            sources: sources.iter().map(PathBuf::from).collect(),
            exclude: vec![],
            include_sensitive: false,
            sync: Some(SyncSettings {
                destination: Destination {
                    account_id: Uuid::new_v4(),
                    path: RelPath::parse("Docs").unwrap(),
                },
                delivery: Delivery::Files,
                mode: SyncMode::Upload,
                schedule: Schedule::Manual,
                guard: GuardLimits::default(),
            }),
        }
    }

    fn issues(job: &Job) -> Vec<String> {
        let app_dirs = [PathBuf::from("/home/u/.config/syncly")];
        job.validate(&ValidationContext {
            app_dirs: &app_dirs,
        })
        .into_iter()
        .map(|i| i.message)
        .collect()
    }

    #[test]
    fn tarefa_valida() {
        assert!(issues(&job(&["/home/u/Documentos", "/home/u/notas.md"])).is_empty());
    }

    #[test]
    fn rn11_rn12_fontes_invalidas() {
        assert!(!issues(&job(&["relativo/x"])).is_empty());
        assert!(!issues(&job(&["/"])).is_empty());
        assert!(!issues(&job(&["/proc/1"])).is_empty());
        assert!(!issues(&job(&["/home/u/../x"])).is_empty());
        assert!(!issues(&job(&["/home/u/.config/syncly/x"])).is_empty());
        assert!(!issues(&job(&[])).is_empty());
    }

    #[test]
    fn rn13_fontes_aninhadas() {
        assert!(!issues(&job(&["/home/u", "/home/u/Docs"])).is_empty());
    }

    #[test]
    fn rn14_nomes_repetidos() {
        assert!(!issues(&job(&["/a/Docs", "/b/docs"])).is_empty());
    }

    #[test]
    fn rn15_precisa_de_um_destino() {
        let mut j = job(&["/home/u/Docs"]);
        j.sync = None;
        assert!(!issues(&j).is_empty());
    }

    #[test]
    fn rn16_destino_nao_pode_ser_a_raiz() {
        let mut j = job(&["/home/u/Docs"]);
        if let Some(s) = j.sync.as_mut() {
            s.destination.path = RelPath::parse("Backups/x").unwrap();
        }
        // "Backups" deixou de ser reservada: tarefas compactadas costumam usá-la.
        assert!(issues(&j).is_empty());
        if let Some(s) = j.sync.as_mut() {
            s.destination.path = RelPath::root();
        }
        assert!(!issues(&j).is_empty());
    }

    #[test]
    fn rn22_fonte_sensivel_exige_marcacao_explicita() {
        let j = job(&["/home/u/.ssh", "/home/u/projeto/.env"]);
        assert_eq!(issues(&j).len(), 2);
        let mut j = j;
        j.include_sensitive = true;
        assert!(issues(&j).is_empty());
        // Exclusão do próprio usuário também vale para a fonte.
        let mut j = job(&["/home/u/notas.md"]);
        j.exclude = vec!["*.md".into()];
        assert_eq!(issues(&j).len(), 1);
    }

    #[test]
    fn rn54_compactado_mantem_ao_menos_uma_versao() {
        let mut j = job(&["/home/u/Docs"]);
        let archive = |keep_last| Delivery::Archive {
            format: ArchiveFormat::TarZst,
            retention: Retention {
                keep_last,
                ..Retention::default()
            },
            encrypt: false,
        };
        if let Some(s) = j.sync.as_mut() {
            s.delivery = archive(0);
        }
        assert!(!issues(&j).is_empty());
        if let Some(s) = j.sync.as_mut() {
            s.delivery = archive(1);
        }
        assert!(issues(&j).is_empty());
    }

    #[test]
    fn rn63_criptografia_so_em_tar_zst() {
        let mut j = job(&["/home/u/Docs"]);
        let with = |format| Delivery::Archive {
            format,
            retention: Retention::default(),
            encrypt: true,
        };
        if let Some(s) = j.sync.as_mut() {
            s.delivery = with(ArchiveFormat::Zip);
        }
        assert!(issues(&j).iter().any(|i| i.contains("TAR.ZST")));
        if let Some(s) = j.sync.as_mut() {
            s.delivery = with(ArchiveFormat::TarZst);
        }
        assert!(issues(&j).is_empty());
    }
}
