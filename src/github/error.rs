//! Errors of the GitHub adapter.

/// An error of a GitHub request. Workers map it to `bus::SyncError`.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GithubError {
    /// No token, or GitHub rejected the token.
    #[error("GitHub authentication failed")]
    Auth,
}
