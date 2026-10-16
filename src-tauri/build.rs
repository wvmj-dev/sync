// SEG-30: todo comando chamado pela webview precisa estar nesta lista E ser
// liberado em capabilities/default.json. Comando fora da lista = bloqueado.
const COMMANDS: &[&str] = &[
    "app_info",
    "system_appearance",
    "get_preferences",
    "save_preferences",
    "encryption_status",
    "create_encryption_key",
    "activate_encryption_key",
    "import_encryption_key",
    "list_accounts",
    "add_local_account",
    "remove_account",
    "list_jobs",
    "save_job",
    "delete_job",
    "preview_job",
    "connect_google",
    "list_remote",
    "create_remote_folder",
    "upload_file",
    "download_file",
    "download_and_extract",
    "upload_compressed",
    "run_job",
    "cancel_run",
    "job_status",
    "refresh_job_status",
    "list_versions",
    "restore_version",
    "compress_paths",
    "extract_archive",
    "cancel_archive",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("falha ao gerar as permissões do app");
}
