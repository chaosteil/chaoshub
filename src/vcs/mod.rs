//! Adapter for jj workspaces and stacks, and git ref prefetch.
#![allow(dead_code, reason = "the workers use the adapter in a later phase")]

pub(crate) mod error;
mod git;
pub(crate) mod jj;
pub(crate) mod lock;
mod parse;

use tokio::process::Command;

use self::error::VcsError;

/// Runs `cmd` and returns its standard output. An exit status other than
/// zero gives an error with the standard error text.
async fn run(mut cmd: Command) -> Result<String, VcsError> {
    let program = cmd.as_std().get_program().to_string_lossy().into_owned();
    let output = cmd.output().await.map_err(|source| VcsError::Spawn {
        program: program.clone(),
        source,
    })?;
    if !output.status.success() {
        return Err(VcsError::Failed {
            program,
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use tokio::process::Command;

    /// The arguments of `cmd` as strings, without the program.
    pub(super) fn args(cmd: &Command) -> Vec<String> {
        cmd.as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }
}
