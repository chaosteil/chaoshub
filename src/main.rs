//! The chaoshub binary.
#![forbid(unsafe_code)]

use anyhow::Context as _;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};

fn main() -> anyhow::Result<()> {
    let _log_guard = init_logging()?;
    tracing::info!("chaoshub started");
    chaoshub::run()
}

/// Sends tracing output to `chaoshub.log` in the state directory. The log
/// is flushed when the returned guard drops.
fn init_logging() -> anyhow::Result<WorkerGuard> {
    let dir = chaoshub::state_dir().context("the state directory is unknown")?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::NEVER)
        .filename_prefix("chaoshub")
        .filename_suffix("log")
        .build(&dir)
        .with_context(|| format!("cannot open the log in {}", dir.display()))?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_ansi(false)
        .init();
    Ok(guard)
}
