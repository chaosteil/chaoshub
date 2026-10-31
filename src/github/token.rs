//! The token source: runs `gh auth token` on first use and caches the token.

use tokio::process::Command;
use tokio::sync::Mutex;

use crate::github::error::GithubError;

/// Supplies the GitHub token. It runs the command only on first use, so
/// no subprocess runs before the first request. After a 401 the client
/// calls [`TokenSource::refresh`] once.
#[derive(Debug)]
pub(crate) struct TokenSource {
    program: String,
    args: Vec<String>,
    cached: Mutex<Option<String>>,
}

impl TokenSource {
    /// A source that runs `gh auth token`.
    pub(crate) fn gh() -> Self {
        Self::command("gh", &["auth", "token"])
    }

    /// A source that runs `program` with `args` and reads the token from
    /// its standard output.
    pub(crate) fn command(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_owned(),
            args: args.iter().map(|&arg| arg.to_owned()).collect(),
            cached: Mutex::new(None),
        }
    }

    /// The cached token. The first call runs the command.
    pub(crate) async fn token(&self) -> Result<String, GithubError> {
        let mut cached = self.cached.lock().await;
        if let Some(token) = cached.as_ref() {
            return Ok(token.clone());
        }
        let token = self.run().await?;
        *cached = Some(token.clone());
        Ok(token)
    }

    /// Runs the command again, because GitHub rejected `stale`. When
    /// another task refreshed the token already, it returns that token
    /// and does not run the command.
    pub(crate) async fn refresh(&self, stale: &str) -> Result<String, GithubError> {
        let mut cached = self.cached.lock().await;
        if let Some(token) = cached.as_ref().filter(|token| token.as_str() != stale) {
            return Ok(token.clone());
        }
        *cached = None;
        let token = self.run().await?;
        *cached = Some(token.clone());
        Ok(token)
    }

    /// Runs the command and returns the trimmed first line of its output.
    async fn run(&self) -> Result<String, GithubError> {
        let output = Command::new(&self.program)
            .args(&self.args)
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|err| {
                tracing::warn!(program = %self.program, %err, "token command did not start");
                GithubError::Auth
            })?;
        if !output.status.success() {
            tracing::warn!(
                program = %self.program,
                status = %output.status,
                stderr = %String::from_utf8_lossy(&output.stderr).trim(),
                "token command failed"
            );
            return Err(GithubError::Auth);
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let token = stdout.lines().next().unwrap_or_default().trim();
        if token.is_empty() {
            tracing::warn!(program = %self.program, "token command printed no token");
            return Err(GithubError::Auth);
        }
        tracing::debug!(program = %self.program, "token read");
        Ok(token.to_owned())
    }
}

#[cfg(test)]
impl TokenSource {
    /// A source whose token is `tok-<n>`, where `n` counts the runs in a
    /// file inside `dir`.
    pub(crate) fn counting(dir: &std::path::Path) -> Self {
        let file = dir.join("runs");
        let script = format!(
            "echo x >> '{0}'; echo tok-$(wc -l < '{0}' | tr -d ' ')",
            file.display()
        );
        Self::command("sh", &["-c", &script])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn token_runs_the_command_once() {
        let dir = tempfile::tempdir().unwrap();
        let source = TokenSource::counting(dir.path());
        assert_eq!(source.token().await.unwrap(), "tok-1");
        assert_eq!(source.token().await.unwrap(), "tok-1");
    }

    #[tokio::test]
    async fn refresh_runs_the_command_again() {
        let dir = tempfile::tempdir().unwrap();
        let source = TokenSource::counting(dir.path());
        let first = source.token().await.unwrap();
        assert_eq!(source.refresh(&first).await.unwrap(), "tok-2");
        assert_eq!(source.token().await.unwrap(), "tok-2");
    }

    #[tokio::test]
    async fn refresh_with_an_old_token_keeps_the_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let source = TokenSource::counting(dir.path());
        let first = source.token().await.unwrap();
        source.refresh(&first).await.unwrap();
        assert_eq!(source.refresh(&first).await.unwrap(), "tok-2");
    }

    #[tokio::test]
    async fn token_trims_the_output() {
        let source = TokenSource::command("sh", &["-c", "printf '  gho_abc  \\n'"]);
        assert_eq!(source.token().await.unwrap(), "gho_abc");
    }

    #[tokio::test]
    async fn failures_are_auth_errors() {
        let cases = [
            ("exit code", TokenSource::command("sh", &["-c", "exit 1"])),
            ("empty output", TokenSource::command("sh", &["-c", "true"])),
            (
                "missing program",
                TokenSource::command("chaoshub-no-such-program", &[]),
            ),
        ];
        for (name, source) in cases {
            assert!(
                matches!(source.token().await, Err(GithubError::Auth)),
                "{name}"
            );
        }
    }
}
