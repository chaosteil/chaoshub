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

impl GithubError {
    /// True when GitHub stopped the request because it took too long: a
    /// 502 or a 504, or a GraphQL error that names a timeout. GitHub
    /// sometimes sends a timeout as a 200 with a GraphQL error.
    pub(crate) fn is_timeout(&self) -> bool {
        match self {
            Self::Http {
                status: Some(502 | 504),
                ..
            } => true,
            Self::Graphql(messages) => messages
                .iter()
                .any(|message| message.to_lowercase().contains("timeout")),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(status: Option<u16>) -> GithubError {
        GithubError::Http {
            status,
            message: "failed".into(),
        }
    }

    #[test]
    fn is_timeout_cases() {
        let cases = [
            ("502 is a timeout", http(Some(502)), true),
            ("504 is a timeout", http(Some(504)), true),
            ("500 is no timeout", http(Some(500)), false),
            ("no response is no timeout", http(None), false),
            (
                "a GraphQL timeout message is a timeout",
                GithubError::Graphql(vec![
                    "Something went wrong while executing your query. This may be the result of a timeout, or it could be a GitHub bug.".into(),
                ]),
                true,
            ),
            (
                "the case of the message does not matter",
                GithubError::Graphql(vec!["Timeout".into()]),
                true,
            ),
            (
                "another GraphQL error is no timeout",
                GithubError::Graphql(vec!["Field 'x' doesn't exist".into()]),
                false,
            ),
            ("auth is no timeout", GithubError::Auth, false),
            (
                "a rate limit is no timeout",
                GithubError::RateLimited { until: Utc::now() },
                false,
            ),
            ("a parse error is no timeout", GithubError::Parse("x".into()), false),
        ];
        for (name, err, timeout) in cases {
            assert_eq!(err.is_timeout(), timeout, "{name}");
        }
    }
}
