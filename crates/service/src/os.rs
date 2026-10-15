//! Integração com o sistema: timers do systemd do usuário, validação de agendamento e
//! notificações (RN-82, RN-94, SEG-22). Fica atrás de uma interface para os testes nunca
//! tocarem o systemd nem a sessão real (AGENTS.md, regra 9) e, no futuro, para launchd
//! (macOS) e Agendador de Tarefas (Windows).

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use async_trait::async_trait;

use crate::error::{ServiceError, ServiceResult};

#[async_trait]
pub trait OsIntegration: Send + Sync {
    /// Executável que os timers chamam (o próprio programa em execução).
    fn exe(&self) -> ServiceResult<PathBuf>;
    /// `systemctl --user <args>` — argumentos fixos, nunca via shell.
    fn systemctl(&self, args: &[&str]) -> ServiceResult<()>;
    /// RN-82: valida uma expressão `OnCalendar` com `systemd-analyze calendar`.
    fn verify_calendar(&self, expr: &str) -> ServiceResult<()>;
    /// RN-94: notificação na área de trabalho.
    async fn notify(&self, title: &str, body: &str) -> ServiceResult<()>;
}

/// Linux com systemd e um servidor de notificações (GNOME, KDE…).
pub struct LinuxDesktop;

fn first_line(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .trim()
        .to_string()
}

#[async_trait]
impl OsIntegration for LinuxDesktop {
    fn exe(&self) -> ServiceResult<PathBuf> {
        let exe = std::env::current_exe().map_err(|e| {
            ServiceError::Unavailable(format!("não foi possível localizar o executável: {e}"))
        })?;
        // Depois de uma atualização, o binário em execução pode ter sido substituído.
        if !exe.is_file() {
            return Err(ServiceError::Unavailable(
                "o executável do Syncly mudou — abra o app de novo para atualizar os agendamentos"
                    .into(),
            ));
        }
        Ok(exe)
    }

    fn systemctl(&self, args: &[&str]) -> ServiceResult<()> {
        let out = Command::new("systemctl")
            .arg("--user")
            .args(args)
            .output()
            .map_err(|e| ServiceError::Unavailable(format!("systemctl indisponível: {e}")))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(ServiceError::Unavailable(format!(
                "systemctl --user {} falhou: {}",
                args.join(" "),
                first_line(&out.stderr)
            )))
        }
    }

    fn verify_calendar(&self, expr: &str) -> ServiceResult<()> {
        let out = Command::new("systemd-analyze")
            .args(["calendar", "--", expr])
            .output()
            .map_err(|e| {
                ServiceError::Unavailable(format!(
                    "systemd-analyze indisponível para validar o agendamento: {e}"
                ))
            })?;
        if out.status.success() {
            Ok(())
        } else {
            Err(ServiceError::Invalid(format!(
                "expressão de agendamento inválida: {}",
                first_line(&out.stderr)
            )))
        }
    }

    async fn notify(&self, title: &str, body: &str) -> ServiceResult<()> {
        let unavailable =
            |e: zbus::Error| ServiceError::Unavailable(format!("notificação indisponível: {e}"));
        let conn = zbus::Connection::session().await.map_err(unavailable)?;
        let hints: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
        conn.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "Syncly",
                0u32,
                "dialog-warning",
                title,
                body,
                Vec::<&str>::new(),
                hints,
                -1i32,
            ),
        )
        .await
        .map_err(unavailable)?;
        Ok(())
    }
}
