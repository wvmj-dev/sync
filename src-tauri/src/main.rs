// Evita uma janela de console extra no Windows em release. NÃO REMOVA.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

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
            app.manage(Service::open(paths)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::list_accounts,
            commands::add_local_account,
            commands::remove_account,
            commands::list_jobs,
            commands::save_job,
            commands::delete_job,
            commands::preview_job,
            commands::connect_google,
            commands::list_remote,
            commands::upload_file,
            commands::download_file,
            commands::run_job,
            commands::cancel_run,
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
