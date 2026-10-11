//! `syncly` — linha de comando. Também é o que os timers do systemd executam (fase 3).

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use syncly_core::sync::guard::GuardVerdict;
use syncly_service::dto::{PlanPreview, RemoteItem, RunReport, human_bytes};
use syncly_service::{AppPaths, Progress, RunOptions, RunTrigger, Service, ServiceError, logging};

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
    /// Executa a sincronização de uma tarefa.
    Run {
        /// Nome ou ID da tarefa.
        job: String,
        /// RN-45: necessário na primeira execução e quando a guarda pede confirmação.
        #[arg(long)]
        confirm: bool,
        /// Usado pelos timers do systemd (RN-43: nunca confirma sozinho).
        #[arg(long, hide = true)]
        scheduled: bool,
    },
    /// Conta Google.
    Google {
        #[command(subcommand)]
        action: GoogleCommand,
    },
    /// Arquivos dentro da pasta Syncly/ de uma conta.
    Remote {
        #[command(subcommand)]
        action: RemoteCommand,
        /// Nome, e-mail ou ID da conta (dispensável se houver só uma).
        #[arg(long, global = true, value_name = "CONTA")]
        account: Option<String>,
    },
}

#[derive(Subcommand)]
enum GoogleCommand {
    /// Conecta (ou reconecta) uma conta: abre o navegador para você autorizar.
    Connect,
}

#[derive(Subcommand)]
enum RemoteCommand {
    /// Lista uma pasta (padrão: a raiz Syncly/).
    Ls {
        #[arg(default_value = "", value_name = "PASTA")]
        path: String,
    },
    /// Envia um arquivo. Nunca sobrescreve um existente.
    Put {
        #[arg(value_name = "ARQUIVO")]
        file: PathBuf,
        #[arg(default_value = "", value_name = "PASTA")]
        dir: String,
    },
    /// Baixa um arquivo. Nunca sobrescreve: usa "nome (2)" se preciso.
    Get {
        #[arg(value_name = "CAMINHO")]
        path: String,
        #[arg(default_value = ".", value_name = "DESTINO")]
        dest: PathBuf,
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
        Command::Run {
            job,
            confirm,
            scheduled,
        } => {
            let id = service.find_job_id(&job)?;
            let trigger = if scheduled {
                RunTrigger::Scheduled
            } else {
                RunTrigger::Manual
            };
            let interactive = std::io::stderr().is_terminal();
            let result = service
                .run_job(id, RunOptions { confirm, trigger }, |p| {
                    if interactive {
                        print_progress(p);
                    }
                })
                .await;
            if interactive {
                eprintln!();
            }
            return match result {
                Ok(report) => {
                    print_report(&report);
                    Ok(match report.status {
                        "success" => ExitCode::SUCCESS,
                        "partial_success" => ExitCode::from(3),
                        _ => ExitCode::FAILURE,
                    })
                }
                Err(ServiceError::NeedsConfirmation(reasons)) => {
                    eprintln!("Exige confirmação:");
                    reasons.iter().for_each(|r| eprintln!("  - {r}"));
                    eprintln!("Revise com `syncly preview {job:?}` e rode de novo com --confirm.");
                    Ok(ExitCode::from(2))
                }
                Err(e) => Err(e),
            };
        }
        Command::Google {
            action: GoogleCommand::Connect,
        } => {
            let account = service
                .connect_google(|url| {
                    println!("Abrindo o navegador para você autorizar o acesso ao Drive.");
                    println!("Se não abrir, copie este endereço no navegador:\n\n  {url}\n");
                    println!("Aguardando autorização (até 5 minutos)…");
                })
                .await?;
            println!(
                "Conta conectada: {} — pasta Syncly/ pronta no seu Drive.",
                account.label
            );
        }
        Command::Remote { action, account } => {
            let id = service.find_account_id(account.as_deref())?;
            match action {
                RemoteCommand::Ls { path } => {
                    let items = service.list_remote(id, &path).await?;
                    if items.is_empty() {
                        println!("(pasta vazia)");
                    }
                    items.iter().for_each(print_item);
                }
                RemoteCommand::Put { file, dir } => {
                    let item = service.upload_file(id, &absolute(&file)?, &dir).await?;
                    println!("Enviado e verificado: Syncly/{}", item.path);
                }
                RemoteCommand::Get { path, dest } => {
                    let saved = service.download_file(id, &path, &absolute(&dest)?).await?;
                    println!("Baixado e verificado: {}", saved.display());
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn print_progress(p: &Progress) {
    let current = p.current.as_deref().unwrap_or("");
    let current: String = current
        .chars()
        .rev()
        .take(50)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    eprint!(
        "\r\x1b[2K[{}/{}] {} de {}  {current}",
        p.done,
        p.total,
        human_bytes(p.bytes_done),
        human_bytes(p.bytes_total)
    );
    let _ = std::io::stderr().flush();
}

fn print_report(r: &RunReport) {
    let label = match r.status {
        "success" => "Concluída",
        "partial_success" => "Concluída com falhas",
        "cancelled" => "Cancelada",
        "aborted" => "Interrompida",
        _ => "Falhou",
    };
    let e = &r.report;
    println!("{label} em {:.1} s", r.duration_ms as f64 / 1000.0);
    println!(
        "  enviados: {}  atualizados: {}  conflitos: {}  pastas: {}  lixeira: {}  ({})",
        e.uploaded,
        e.updated,
        e.conflicts,
        e.folders_created,
        e.trashed,
        human_bytes(e.bytes_sent)
    );
    if e.unstable > 0 {
        println!(
            "  {} arquivo(s) mudaram durante o envio e vão de novo na próxima execução",
            e.unstable
        );
    }
    if let Some(reason) = &e.aborted {
        println!("  motivo: {reason}");
    }
    if r.failure_count > 0 {
        println!("  {} falha(s):", r.failure_count);
        for f in e.failures.iter().take(10) {
            println!("    {}: {}", f.path, f.error);
        }
    }
}

fn absolute(path: &std::path::Path) -> Result<PathBuf, ServiceError> {
    std::fs::canonicalize(path)
        .map_err(|e| ServiceError::Invalid(format!("'{}': {e}", path.display())))
}

fn print_item(item: &RemoteItem) {
    if item.kind == "folder" {
        println!("  {}/", item.name);
        return;
    }
    let size = item.size.map(human_bytes).unwrap_or_default();
    let when = item
        .modified_at
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%d/%m/%Y %H:%M")
                .to_string()
        })
        .unwrap_or_default();
    let who = item.modified_by.as_deref().unwrap_or("");
    let note = if item.downloadable {
        ""
    } else {
        "  (não baixável)"
    };
    println!("  {:<40} {:>10}  {when}  {who}{note}", item.name, size);
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
