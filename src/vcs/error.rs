//! Errors of the vcs adapter.

use std::path::PathBuf;
use std::process::ExitStatus;

/// A failure of a jj or git process, or a repo that chaoshub cannot read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum VcsError {
    /// The process did not start, for example because the program is
    /// missing.
    #[error("cannot start {program}: {source}")]
    Spawn {
        program: String,
        source: std::io::Error,
    },
    /// The process exited with a status other than zero.
    #[error("{program} failed with {status}: {stderr}")]
    Failed {
        program: String,
        status: ExitStatus,
        stderr: String,
    },
    /// A line of the output does not match the template.
    #[error("cannot parse the {what} line: {line:?}")]
    Parse { what: &'static str, line: String },
    /// The repo is a jj repo without `.git`. chaoshub reads only
    /// colocated jj repos.
    #[error(
        "{} is a jj repo without .git. chaoshub needs a colocated repo (`jj git init --colocate`)",
        path.display()
    )]
    PlainJj { path: PathBuf },
}
