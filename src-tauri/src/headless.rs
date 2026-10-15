//! Modo sem janela: `syncly-desktop run <id-da-tarefa> --scheduled`, chamado pelos timers
//! do systemd (RN-80). Roda antes do Tauri — não abre janela nem passa pela instância
//! única (SEG-34). Códigos de saída iguais aos da CLI.

use syncly_service::{AppPaths, Service};
use uuid::Uuid;

const USAGE: &str = "uso: syncly-desktop run <id-da-tarefa> --scheduled";

/// `Some(código de saída)` se os argumentos pedem o modo sem janela.
pub fn run(paths: AppPaths, args: &[String]) -> Option<i32> {
    match args {
        [cmd, id, flag] if cmd == "run" && flag == "--scheduled" => Some(scheduled(paths, id)),
        [cmd, ..] if cmd == "run" => {
            eprintln!("{USAGE}");
            Some(2)
        }
        _ => None,
    }
}

fn scheduled(paths: AppPaths, id: &str) -> i32 {
    let Ok(id) = Uuid::parse_str(id) else {
        eprintln!("{USAGE}");
        return 2;
    };
    let service = match Service::open(paths) {
        Ok(service) => service,
        Err(e) => {
            tracing::error!("execução agendada: {e}");
            return 1;
        }
    };
    // RN-43, RN-94: nunca confirma sozinha; avisa por notificação quando precisa.
    match tauri::async_runtime::block_on(service.run_scheduled(id)) {
        Ok(report) => match report.status {
            "success" => 0,
            "partial_success" => 3,
            _ => 1,
        },
        Err(e) if e.code() == "needs_confirmation" => 2,
        Err(e) => {
            tracing::warn!("execução agendada: {e}");
            1
        }
    }
}
