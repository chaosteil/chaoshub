//! The chaoshub binary.
#![forbid(unsafe_code)]

use anyhow::Context as _;
use chaoshub::ThemeKind;
use chaoshub::cli::Cli;
use clap::Parser as _;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let _log_guard = init_logging()?;
    tracing::info!("chaoshub started");
    let colorterm = std::env::var("COLORTERM").unwrap_or_default();
    let term = std::env::var("TERM").unwrap_or_default();
    chaoshub::run(cli, theme_kind(&colorterm, &term))
}

/// The color support of the terminal from `COLORTERM` and `TERM`: true
/// color when `COLORTERM` is `truecolor` or `24bit`, or when `TERM` names a
/// direct-color terminal. Else the 16 ANSI colors.
fn theme_kind(colorterm: &str, term: &str) -> ThemeKind {
    let colorterm = colorterm.to_ascii_lowercase();
    let term = term.to_ascii_lowercase();
    if colorterm == "truecolor"
        || colorterm == "24bit"
        || term.contains("truecolor")
        || term.contains("24bit")
        || term.ends_with("-direct")
    {
        ThemeKind::TrueColor
    } else {
        ThemeKind::Ansi16
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_kind_cases() {
        let cases = [
            ("truecolor", "xterm-256color", ThemeKind::TrueColor),
            ("24bit", "screen", ThemeKind::TrueColor),
            ("TrueColor", "", ThemeKind::TrueColor),
            ("", "xterm-direct", ThemeKind::TrueColor),
            ("", "xterm-truecolor", ThemeKind::TrueColor),
            ("", "xterm-256color", ThemeKind::Ansi16),
            ("", "linux", ThemeKind::Ansi16),
            ("", "", ThemeKind::Ansi16),
        ];
        for (colorterm, term, kind) in cases {
            assert_eq!(theme_kind(colorterm, term), kind, "{colorterm} {term}");
        }
    }
}
