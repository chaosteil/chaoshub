//! Errors of the launch adapter.

use std::process::ExitStatus;

/// A failure of a launcher command.
#[derive(Debug, thiserror::Error)]
pub(crate) enum LaunchError {
    /// The launcher binary did not start, for example because it is
    /// missing.
    #[error("cannot start {program}: {source}")]
    Spawn {
        program: String,
        source: std::io::Error,
    },
    /// A launcher command exited with a status other than zero.
    #[error("rex {command} failed with {status}: {stderr}")]
    Failed {
        command: &'static str,
        status: ExitStatus,
        stderr: String,
    },
}
