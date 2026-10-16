// Evita uma janela de console extra no Windows em release. NÃO REMOVA.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod appearance;
mod commands;
mod headless;

use syncly_service::{AppPaths, Service, logging};
use tauri::Manager;

fn main() {
    let paths = match AppPaths::discover().and_then(|p| p.ensure().map(|()| p)) {
        Ok(paths) => paths,
        Err(e) => {
            eprintln!("Syncly: {e}");
            std::process::exit(1);
        }
    };
    let log_guard = logging::init(Some(&paths.log_dir), "syncly-desktop", "info");

    // Chamado por um timer do systemd: executa a tarefa e sai, sem abrir janela.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = headless::run(paths.clone(), &args) {
        drop(log_guard); // `exit` não roda destrutores: descarrega o log antes.
        std::process::exit(code);
    }

    let result = tauri::Builder::default()
        // SEG-34: precisa ser o primeiro plugin registrado.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            let service = Service::open(paths)?;
            // Os timers apontam para este executável; atualiza se ele mudou (nova versão).
            if let Err(e) = service.reconcile_schedules() {
                tracing::warn!("agendamentos não atualizados: {e}");
            }
            app.manage(service);
            // Acompanha o tema do sistema (claro/escuro, cor de destaque).
            tauri::async_runtime::spawn(appearance::watch(app.handle().clone()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::system_appearance,
            commands::get_preferences,
            commands::save_preferences,
            commands::list_accounts,
            commands::add_local_account,
            commands::remove_account,
            commands::list_jobs,
            commands::save_job,
            commands::delete_job,
            commands::preview_job,
            commands::connect_google,
            commands::list_remote,
            commands::create_remote_folder,
            commands::upload_file,
            commands::download_file,
            commands::download_and_extract,
            commands::upload_compressed,
            commands::run_job,
            commands::cancel_run,
            commands::job_status,
            commands::refresh_job_status,
            commands::list_versions,
            commands::restore_version,
            commands::compress_paths,
            commands::extract_archive,
            commands::cancel_archive,
        ])
        .run(tauri::generate_context!());

    if let Err(e) = result {
        tracing::error!("falha ao executar o app: {e}");
        drop(log_guard); // `exit` não roda destrutores: descarrega o log antes.
        std::process::exit(1);
    }
}
