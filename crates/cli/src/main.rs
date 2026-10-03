//! `syncly` — linha de comando. Também é o que os timers do systemd executam (fase 3).

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use syncly_core::sync::guard::GuardVerdict;
use syncly_service::dto::{PlanPreview, human_bytes};
use syncly_service::{AppPaths, Service, logging};

#[derive(Parser)]
#[command(name = "syncly", version, about = "Sincronização e backup de pastas")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Mostra onde ficam a configuração e os dados.
    Info,
    /// Lista as contas.
    Accounts,
    /// Lista as tarefas.
    Jobs,
    /// Mostra o que uma sincronização faria, sem alterar nada.
    Preview {
        /// Nome ou ID da tarefa.
        job: String,
        /// Lista todas as ações (padrão: 50).
        #[arg(long)]
        all: bool,
    },
    /// Executa uma tarefa (fase 1).
    Run {
        job: String,
        /// RN-45: necessário quando a guarda pede confirmação.
        #[arg(long)]
        confirm: bool,
    },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(code) => code,
        Err(e) => {
            eprintln!("erro: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<ExitCode, syncly_service::ServiceError> {
    let paths = AppPaths::discover()?;
    paths.ensure()?;
    // Terminal só com avisos; detalhes vão para o arquivo de log.
    let _log = logging::init(Some(&paths.log_dir), "syncly-cli", "warn");
    let service = Service::open(paths)?;
    match cli.command {
        Command::Info => {
            let info = service.app_info();
            println!("Syncly {}", info.version);
            println!("config:  {}", info.config_file);
            println!("dados:   {}", info.data_dir);
            println!("logs:    {}", info.log_dir);
            let google = if info.google_client_configured {
                "configurado"
            } else {
                "não configurado (docs/CONFIGURAR_GOOGLE.md)"
            };
            println!("google:  {google}");
            for problem in [info.google_client_problem, info.config_problem]
                .into_iter()
                .flatten()
            {
                println!("atenção: {problem}");
            }
        }
        Command::Accounts => {
            for a in service.list_accounts()? {
                println!("{}  {:<20} {:<13} {}", a.id, a.label, a.provider, a.detail);
            }
        }
        Command::Jobs => {
            for j in service.list_jobs()? {
                let status = if j.enabled { "ativa" } else { "desativada" };
                let dest = j.sync.as_ref().map_or_else(
                    || "-".to_string(),
                    |s| {
                        format!(
                            "{} → {}:{}",
                            mode_label(s.mode),
                            s.account_label.as_deref().unwrap_or("?"),
                            s.path
                        )
                    },
                );
                println!("{}  {:<20} {:<10} {}", j.id, j.name, status, dest);
            }
        }
        Command::Preview { job, all } => {
            let id = service.find_job_id(&job)?;
            let preview = service.preview_job(id).await?;
            print_preview(&preview, if all { usize::MAX } else { 50 });
        }
        Command::Run { job, confirm: _ } => {
            service.find_job_id(&job)?;
            eprintln!("A execução chega na fase 1. Por enquanto use: syncly preview {job:?}");
            return Ok(ExitCode::from(2));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn mode_label(mode: syncly_core::domain::SyncMode) -> &'static str {
    match mode {
        syncly_core::domain::SyncMode::Upload => "envio",
        syncly_core::domain::SyncMode::Mirror => "espelho",
    }
}

fn print_preview(p: &PlanPreview, limit: usize) {
    let s = &p.stats;
    println!(
        "Plano ({} ms, {} arquivo(s) com hash recalculado):",
        p.duration_ms, p.hashed_files
    );
    println!("  pastas a criar:   {}", s.folders_to_create);
    println!(
        "  enviar:           {} ({})",
        s.uploads,
        human_bytes(s.bytes_to_send)
    );
    println!("  atualizar:        {}", s.updates);
    println!("  conflitos:        {}", s.conflicts);
    println!(
        "  lixeira:          {} arquivo(s), {} pasta(s)",
        s.trash_files, s.trash_folders
    );
    println!("  já idênticos:     {}", s.adopted);
    println!("  ignorados:        {}", s.skipped);

    for a in p.actions.iter().take(limit) {
        match &a.detail {
            Some(d) => println!("  {:<13} {}  ({d})", a.kind, a.path),
            None => println!("  {:<13} {}", a.kind, a.path),
        }
    }
    if p.total_actions > limit.min(p.actions.len()) {
        println!("  … {} ação(ões) no total", p.total_actions);
    }
    for w in &p.warnings {
        println!("aviso: {w}");
    }
    match &p.verdict {
        GuardVerdict::Proceed => println!("Pronto para executar."),
        GuardVerdict::NeedsConfirmation(r) => {
            println!("Exige confirmação (--confirm):");
            r.iter().for_each(|x| println!("  - {x}"));
        }
        GuardVerdict::Abort(r) => {
            println!("BLOQUEADO:");
            r.iter().for_each(|x| println!("  - {x}"));
        }
    }
}
