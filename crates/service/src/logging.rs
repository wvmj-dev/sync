//! Logs em arquivo com rotação diária + terminal (ARQUITETURA.md §5).
//! SEG-52: nunca registrar tokens, cabeçalhos de autorização, chaves ou conteúdo de arquivos.

use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt};

/// Dias de log mantidos.
pub const RETENTION_FILES: usize = 14;
/// Sobrescreve os níveis, ex.: `SYNCLY_LOG=debug`.
pub const ENV_VAR: &str = "SYNCLY_LOG";

/// Inicializa os logs. Mantenha o retorno vivo até o fim do processo — ao ser
/// descartado, ele grava o que ainda estiver no buffer.
///
/// - `prefix`: nome dos arquivos (`syncly-desktop`, `syncly-cli`).
/// - `stderr_level`: nível padrão no terminal (o arquivo usa `info`).
pub fn init(log_dir: Option<&Path>, prefix: &str, stderr_level: &str) -> Option<WorkerGuard> {
    let filter = |default: &str| {
        EnvFilter::try_from_env(ENV_VAR).unwrap_or_else(|_| EnvFilter::new(default))
    };
    let stderr = fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(filter(stderr_level));

    let appender = log_dir.and_then(|dir| {
        Builder::new()
            .rotation(Rotation::DAILY)
            .filename_prefix(prefix)
            .filename_suffix("log")
            .max_log_files(RETENTION_FILES)
            .build(dir)
            .ok()
    });

    let Some(appender) = appender else {
        let _ = tracing_subscriber::registry().with(stderr).try_init();
        if log_dir.is_some() {
            tracing::warn!("não foi possível abrir o arquivo de log; usando só o terminal");
        }
        return None;
    };
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let file = fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_filter(filter("info"));
    let _ = tracing_subscriber::registry()
        .with(stderr)
        .with(file)
        .try_init();
    Some(guard)
}
