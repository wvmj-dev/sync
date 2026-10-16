//! Agendamento pelo systemd do usuário (RN-18, RN-36, RN-80 a RN-85, SEG-22): gera as
//! units de cada tarefa e calcula o que mudar nas instaladas. Tudo puro — quem grava os
//! arquivos e chama o `systemctl` é o serviço.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use uuid::Uuid;

use crate::domain::{Job, Schedule};

/// Primeira linha de toda unit do app. Só arquivos com ela são atualizados ou removidos —
/// uma unit criada à mão com o mesmo nome nunca é tocada.
pub const HEADER: &str = "# Gerado pelo Syncly — não edite: é recriado a cada alteração da tarefa.";
const PREFIX: &str = "syncly-";

/// RN-84: código de saída de uma execução agendada adiada que deve ser tentada de novo
/// (`EX_TEMPFAIL`). A unit pede ao systemd que a reinicie depois de [`RETRY_MINUTES`].
pub const EXIT_RETRY: u8 = 75;
/// RN-84: espera entre as tentativas sem rede.
pub const RETRY_MINUTES: u32 = 15;
/// RN-84: sem rede, tenta de novo até completar esta quantidade de adiamentos seguidos
/// (cerca de 3 h); depois, só no próximo horário agendado.
pub const MAX_RETRIES: u32 = 12;
/// RN-84: no adiamento seguido de número 3, avisa — uma vez só.
pub const NOTIFY_AT: u32 = 3;

/// RN-84: por que uma execução agendada foi adiada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostponeReason {
    /// Sem conexão com o provedor.
    Offline,
    /// O cofre de senhas do sistema está bloqueado ou indisponível.
    KeyringLocked,
}

/// RN-84: o que fazer com um adiamento.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Postponement {
    /// Pedir ao systemd outra tentativa em [`RETRY_MINUTES`].
    pub retry: bool,
    pub notify: bool,
}

/// RN-84: `streak` conta este adiamento e os anteriores seguidos. Com o cofre bloqueado
/// não há nova tentativa automática: cada uma abriria de novo o pedido de senha na tela.
pub fn postponement(reason: PostponeReason, streak: u32) -> Postponement {
    Postponement {
        retry: reason == PostponeReason::Offline && streak < MAX_RETRIES,
        notify: streak == NOTIFY_AT,
    }
}

/// Situação do agendamento de uma tarefa, para a interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerState {
    /// Sem agendamento.
    Manual,
    /// RN-18: tarefa desativada não tem timer.
    Disabled,
    /// RN-85: a conta precisa ser reconectada; volta sozinho ao reconectar.
    Paused,
    /// RN-36: só ativa depois da primeira execução manual bem-sucedida.
    WaitingFirstRun,
    Active,
    /// Deveria estar ativo, mas a unit não está instalada (systemd indisponível?).
    NotInstalled,
}

impl TimerState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Disabled => "disabled",
            Self::Paused => "paused",
            Self::WaitingFirstRun => "waiting_first_run",
            Self::Active => "active",
            Self::NotInstalled => "not_installed",
        }
    }
}

/// RN-18, RN-36, RN-85: a tarefa deve ter timer? `has_success`: já houve uma execução
/// bem-sucedida do tipo que ela faz; `account_ok`: a conta não precisa ser reconectada.
pub fn wants_timer(job: &Job, has_success: bool, account_ok: bool) -> bool {
    timer_state(job, has_success, account_ok, true) == TimerState::Active
}

pub fn timer_state(job: &Job, has_success: bool, account_ok: bool, installed: bool) -> TimerState {
    let scheduled = job
        .sync
        .as_ref()
        .is_some_and(|s| s.schedule != Schedule::Manual);
    if !scheduled {
        TimerState::Manual
    } else if !job.enabled {
        TimerState::Disabled
    } else if !account_ok {
        TimerState::Paused
    } else if !has_success {
        TimerState::WaitingFirstRun
    } else if installed {
        TimerState::Active
    } else {
        TimerState::NotInstalled
    }
}

/// `syncly-<uuid>` — nome base das units de uma tarefa.
pub fn unit_base(job_id: Uuid) -> String {
    format!("{PREFIX}{job_id}")
}

/// Nome de arquivo de unit do app (`syncly-<uuid>.service` ou `.timer`).
pub fn is_unit_name(name: &str) -> bool {
    name.strip_prefix(PREFIX)
        .and_then(|rest| {
            rest.strip_suffix(".service")
                .or_else(|| rest.strip_suffix(".timer"))
        })
        .is_some_and(|id| Uuid::parse_str(id).is_ok())
}

/// SEG-22: o executável vai entre aspas e com `%` escapado (especificador do systemd);
/// caracteres que poderiam quebrar a linha ou as aspas são recusados.
fn exec_path(exe: &Path) -> Result<String, String> {
    let text = exe.to_str().ok_or("o caminho do executável não é UTF-8")?;
    if !exe.is_absolute() {
        return Err(format!("'{text}' não é um caminho absoluto"));
    }
    if text
        .chars()
        .any(|c| c.is_control() || c == '"' || c == '\\')
    {
        return Err(format!("'{text}' tem caracteres não permitidos numa unit"));
    }
    Ok(format!("\"{}\"", text.replace('%', "%%")))
}

/// RN-80, RN-83, RN-84: as units (`.service` e `.timer`) de uma tarefa agendada. Só entram
/// valores gerados pelo app: o UUID, o executável e o `OnCalendar` validado (SEG-22).
pub fn render_units(
    job_id: Uuid,
    schedule: &Schedule,
    exe: &Path,
) -> Result<Vec<(String, String)>, String> {
    schedule.validate()?;
    let on_calendar = schedule
        .on_calendar()
        .ok_or("a tarefa não tem agendamento")?;
    let base = unit_base(job_id);
    let exec = exec_path(exe)?;
    let service = format!(
        "{HEADER}\n[Unit]\nDescription=Syncly: tarefa {job_id}\n\n\
         [Service]\nType=oneshot\nExecStart={exec} run {job_id} --scheduled\n\
         RestartForceExitStatus={EXIT_RETRY}\nRestartSec={RETRY_MINUTES}min\n\
         Nice=10\nIOSchedulingClass=idle\n"
    );
    let timer = format!(
        "{HEADER}\n[Unit]\nDescription=Syncly: agendamento da tarefa {job_id}\n\n\
         [Timer]\nOnCalendar={on_calendar}\nPersistent=true\nRandomizedDelaySec=2min\n\
         Unit={base}.service\n\n[Install]\nWantedBy=timers.target\n"
    );
    Ok(vec![
        (format!("{base}.service"), service),
        (format!("{base}.timer"), timer),
    ])
}

/// O que mudar nas units instaladas.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TimerPlan {
    /// Timers a desativar antes de remover os arquivos.
    pub stop: Vec<String>,
    /// Arquivos do app que não deveriam mais existir.
    pub remove: Vec<String>,
    /// Arquivos a gravar (novos ou alterados).
    pub write: Vec<(String, String)>,
    /// Timers a (re)ativar depois de gravar.
    pub start: Vec<String>,
}

impl TimerPlan {
    pub fn is_empty(&self) -> bool {
        self.write.is_empty() && self.remove.is_empty()
    }
}

/// Compara as units desejadas com as instaladas (nome → conteúdo). Arquivos sem
/// [`HEADER`] nunca são sobrescritos nem removidos. Idempotente: com tudo igual, o plano
/// sai vazio.
pub fn plan_timers(
    desired: &BTreeMap<String, String>,
    installed: &BTreeMap<String, String>,
) -> TimerPlan {
    let ours = |content: &str| content.starts_with(HEADER);
    let mut plan = TimerPlan::default();
    for (name, content) in desired {
        match installed.get(name) {
            Some(current) if current == content || !ours(current) => {}
            _ => plan.write.push((name.clone(), content.clone())),
        }
    }
    for (name, _) in &plan.write {
        let timer = match name.strip_suffix(".service") {
            Some(base) => format!("{base}.timer"),
            None => name.clone(),
        };
        if desired.contains_key(&timer) && !plan.start.contains(&timer) {
            plan.start.push(timer);
        }
    }
    for (name, content) in installed {
        if !desired.contains_key(name) && ours(content) {
            if name.ends_with(".timer") {
                plan.stop.push(name.clone());
            }
            plan.remove.push(name.clone());
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Delivery, Destination, GuardLimits, RelPath, SyncMode, SyncSettings};
    use std::path::PathBuf;

    fn daily() -> Schedule {
        Schedule::Daily {
            at: "02:30".to_string().try_into().unwrap(),
        }
    }

    fn job(enabled: bool, schedule: Schedule) -> Job {
        Job {
            id: Uuid::nil(),
            name: "Docs".into(),
            enabled,
            sources: vec!["/home/u/Docs".into()],
            exclude: vec![],
            include_sensitive: false,
            sync: Some(SyncSettings {
                destination: Destination {
                    account_id: Uuid::nil(),
                    path: RelPath::parse("Docs").unwrap(),
                },
                delivery: Delivery::Files,
                mode: SyncMode::Upload,
                schedule,
                guard: GuardLimits::default(),
            }),
        }
    }

    #[test]
    fn rn18_rn36_rn85_quem_tem_timer() {
        assert_eq!(
            timer_state(&job(true, Schedule::Manual), true, true, false),
            TimerState::Manual
        );
        assert_eq!(
            timer_state(&job(false, daily()), true, true, true),
            TimerState::Disabled
        );
        assert_eq!(
            timer_state(&job(true, daily()), false, true, false),
            TimerState::WaitingFirstRun
        );
        assert_eq!(
            timer_state(&job(true, daily()), true, true, false),
            TimerState::NotInstalled
        );
        assert!(wants_timer(&job(true, daily()), true, true));
        assert!(!wants_timer(&job(true, daily()), false, true));
        assert!(!wants_timer(&job(false, daily()), true, true));
        // RN-85: conta a reconectar pausa o timer.
        assert_eq!(
            timer_state(&job(true, daily()), true, false, true),
            TimerState::Paused
        );
        assert!(!wants_timer(&job(true, daily()), true, false));
    }

    #[test]
    fn rn83_seg22_units_so_com_valores_do_app() {
        let id = Uuid::nil();
        let units = render_units(id, &daily(), Path::new("/usr/bin/syncly 100%")).unwrap();
        let (service_name, service) = &units[0];
        let (timer_name, timer) = &units[1];
        assert_eq!(service_name, &format!("syncly-{id}.service"));
        assert!(service.starts_with(HEADER));
        assert!(service.contains(&format!(
            "ExecStart=\"/usr/bin/syncly 100%%\" run {id} --scheduled\n"
        )));
        assert_eq!(timer_name, &format!("syncly-{id}.timer"));
        assert!(timer.contains("OnCalendar=*-*-* 02:30:00\n"));
        assert!(timer.contains("Persistent=true\n"));
        // RN-84: adiada sem rede → o systemd tenta de novo.
        assert!(service.contains("RestartForceExitStatus=75\nRestartSec=15min\n"));
        // Nome da tarefa (texto do usuário) nunca entra na unit.
        assert!(!service.contains("Docs") && !timer.contains("Docs"));

        for bad in [
            "relativo/syncly",
            "/usr/bin/sync\"ly",
            "/usr/bin/a\nExecStart=/bin/sh",
        ] {
            assert!(
                render_units(id, &daily(), &PathBuf::from(bad)).is_err(),
                "{bad}"
            );
        }
        let injected = Schedule::Calendar {
            expr: "daily\nExecStart=/bin/sh".into(),
        };
        assert!(render_units(id, &injected, Path::new("/usr/bin/syncly")).is_err());
        assert!(render_units(id, &Schedule::Manual, Path::new("/usr/bin/syncly")).is_err());
    }

    #[test]
    fn rn84_adiamento_tenta_de_novo_sem_rede_e_avisa_no_terceiro() {
        use PostponeReason::{KeyringLocked, Offline};
        let first = postponement(Offline, 1);
        assert!(first.retry && !first.notify);
        // Avisa no 3º seguido — e só nele, para não repetir a cada 15 min.
        assert!(postponement(Offline, NOTIFY_AT).notify);
        assert!(!postponement(Offline, NOTIFY_AT + 1).notify);
        // Sem rede por muito tempo: para de insistir e espera o próximo horário.
        assert!(postponement(Offline, MAX_RETRIES - 1).retry);
        assert!(!postponement(Offline, MAX_RETRIES).retry);
        // Cofre bloqueado: nunca tenta sozinho (cada tentativa pediria a senha de novo).
        assert!(!postponement(KeyringLocked, 1).retry);
        assert!(postponement(KeyringLocked, NOTIFY_AT).notify);
    }

    #[test]
    fn reconhece_so_nomes_de_unit_do_app() {
        let id = Uuid::new_v4();
        assert!(is_unit_name(&format!("syncly-{id}.timer")));
        assert!(is_unit_name(&format!("syncly-{id}.service")));
        for other in [
            "syncly-x.timer",
            "outro.timer",
            &format!("syncly-{id}.socket"),
        ] {
            assert!(!is_unit_name(other), "{other}");
        }
    }

    #[test]
    fn reconciliacao_idempotente_e_respeita_units_alheias() {
        let a = Uuid::new_v4();
        let units = render_units(a, &daily(), Path::new("/usr/bin/syncly")).unwrap();
        let desired: BTreeMap<_, _> = units.into_iter().collect();

        // Nada instalado: grava as duas e ativa o timer.
        let plan = plan_timers(&desired, &BTreeMap::new());
        assert_eq!(plan.write.len(), 2);
        assert_eq!(plan.start, [format!("syncly-{a}.timer")]);
        assert!(plan.stop.is_empty() && plan.remove.is_empty());

        // Tudo igual: plano vazio.
        assert!(plan_timers(&desired, &desired).is_empty());

        // Tarefa sumiu: desativa e remove as dela; a unit sem cabeçalho fica.
        let b = Uuid::new_v4();
        let mut installed = desired.clone();
        installed.insert(format!("syncly-{b}.timer"), "[Timer]\n# minha\n".into());
        let plan = plan_timers(&BTreeMap::new(), &installed);
        assert_eq!(plan.stop, [format!("syncly-{a}.timer")]);
        assert_eq!(plan.remove.len(), 2);
        assert!(!plan.remove.iter().any(|n| n.contains(&b.to_string())));

        // Unit alheia com o mesmo nome de uma desejada: não é sobrescrita.
        let mut foreign = BTreeMap::new();
        foreign.insert(format!("syncly-{a}.timer"), "[Timer]\n".to_string());
        let plan = plan_timers(&desired, &foreign);
        assert_eq!(plan.write.len(), 1);
        assert!(plan.write[0].0.ends_with(".service"));
    }
}
