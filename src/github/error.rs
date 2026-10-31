//! Errors of the GitHub adapter.

use chrono::{DateTime, Utc};

/// An error of a GitHub request. Workers map it to `bus::SyncError`.
#[derive(Debug, thiserror::Error)]
pub(crate) enum GithubError {
    /// No token, or GitHub rejected the token after one refresh.
    #[error("GitHub authentication failed")]
    Auth,
    /// GitHub limits the requests until `until`.
    #[error("GitHub rate limit until {until}")]
    RateLimited { until: DateTime<Utc> },
    /// A failed request. `status` is `None` when no response came back,
    /// for example when the network is down.
    #[error("GitHub request failed ({}): {message}", status.map_or_else(|| "no response".to_owned(), |s| s.to_string()))]
    Http {
        status: Option<u16>,
        message: String,
    },
    /// The GraphQL response has errors and no data.
    #[error("GitHub GraphQL errors: {}", .0.join("; "))]
    Graphql(Vec<String>),
    /// The response does not have the expected form.
    #[error("unexpected GitHub response: {0}")]
    Parse(String),
}

impl From<reqwest::Error> for GithubError {
    fn from(err: reqwest::Error) -> Self {
        Self::Http {
            status: err.status().map(|status| status.as_u16()),
            message: err.to_string(),
        }
    }
}
