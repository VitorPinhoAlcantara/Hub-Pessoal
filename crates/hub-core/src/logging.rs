use std::path::Path;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;

#[derive(Debug, thiserror::Error)]
pub enum LoggingError {
    #[error("falha ao criar o arquivo de log: {0}")]
    Appender(#[from] tracing_appender::rolling::InitError),
    #[error("falha ao iniciar os logs: {0}")]
    Subscriber(String),
}

const DEFAULT_FILTER: &str =
    "info,iced_wgpu=warn,iced_winit=warn,wgpu_core=warn,wgpu_hal=warn,naga=warn";

pub fn init(logs_dir: &Path) -> Result<WorkerGuard, LoggingError> {
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("hub")
        .filename_suffix("log")
        .max_log_files(7)
        .build(logs_dir)?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let filter =
        EnvFilter::try_from_env("HUB_LOG").unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .try_init()
        .map_err(|error| LoggingError::Subscriber(error.to_string()))?;
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!(%info, %backtrace, "o hub encerrou por um erro interno");
        default_hook(info);
    }));
    Ok(guard)
}
