//! Adapter for the GitHub GraphQL and REST APIs.
#![allow(dead_code, reason = "the workers use the client in a later phase")]

pub(crate) mod budget;
pub(crate) mod error;
mod graphql;
pub(crate) mod parse;
mod queries;
pub(crate) mod rest;
pub(crate) mod token;

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use reqwest::header::HeaderMap;
use reqwest::{RequestBuilder, Response, StatusCode};

pub(crate) use crate::github::error::GithubError;
pub(crate) use crate::github::queries::ReviewEvent;
pub(crate) use crate::github::token::TokenSource;

/// The API root of github.com.
pub(crate) const GITHUB_API: &str = "https://api.github.com";

/// The wait after a rate limit response without a reset time.
const DEFAULT_RATE_WAIT: TimeDelta = TimeDelta::seconds(60);

/// The GitHub client. It is `Send + Sync`; callers share it in an `Arc`.
/// It knows nothing about the UI, the store or the workers.
#[derive(Debug)]
pub(crate) struct GithubClient {
    http: reqwest::Client,
    token: TokenSource,
    /// The API root without a trailing slash, for example
    /// `https://api.github.com`.
    base_url: String,
}

impl GithubClient {
    /// A client for the API at `base_url` that reads its token from
    /// `token`.
    pub(crate) fn new(token: TokenSource, base_url: &str) -> Result<Self, GithubError> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("chaoshub/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            http,
            token,
            base_url: base_url.trim_end_matches('/').to_owned(),
        })
    }

    /// The absolute URL of an API path that starts with `/`.
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    /// Sends the request that `build` makes with the token and maps a
    /// failed status to an error. See [`GithubClient::send_raw`].
    async fn send<F>(&self, build: F) -> Result<Response, GithubError>
    where
        F: Fn(&reqwest::Client) -> RequestBuilder,
    {
        let response = self.send_raw(build).await?;
        if response.status().is_success() {
            return Ok(response);
        }
        Err(status_error(response).await)
    }

    /// Sends the request that `build` makes with the token. After a 401 it
    /// refreshes the token once and sends again; a second 401 is
    /// `GithubError::Auth`. Other statuses come back unchanged.
    async fn send_raw<F>(&self, build: F) -> Result<Response, GithubError>
    where
        F: Fn(&reqwest::Client) -> RequestBuilder,
    {
        let token = self.token.token().await?;
        let response = build(&self.http).bearer_auth(&token).send().await?;
        if response.status() != StatusCode::UNAUTHORIZED {
            return Ok(response);
        }
        tracing::info!("GitHub returned 401, refreshing the token");
        let token = self.token.refresh(&token).await?;
        let response = build(&self.http).bearer_auth(&token).send().await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            return Err(GithubError::Auth);
        }
        Ok(response)
    }
}

/// The error of a response with a failed status: a rate limit, else
/// `Http` with the message of the body.
async fn status_error(response: Response) -> GithubError {
    let status = response.status();
    if let Some(until) = rate_limited_until(status, response.headers(), Utc::now()) {
        tracing::warn!(%status, %until, "GitHub rate limit");
        return GithubError::RateLimited { until };
    }
    let body = response.text().await.unwrap_or_default();
    GithubError::Http {
        status: Some(status.as_u16()),
        message: error_message(&body),
    }
}

/// The end of a rate limit for a 403 or 429 response. `None` when the
/// response is not a rate limit. It honors `Retry-After` first, then
/// `x-ratelimit-reset` when `x-ratelimit-remaining` is 0.
fn rate_limited_until(
    status: StatusCode,
    headers: &HeaderMap,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    if status != StatusCode::FORBIDDEN && status != StatusCode::TOO_MANY_REQUESTS {
        return None;
    }
    if let Some(secs) = header_number(headers, "retry-after") {
        return Some(now + TimeDelta::seconds(secs));
    }
    if header_number(headers, "x-ratelimit-remaining") == Some(0) {
        return Some(reset_at(headers).unwrap_or(now + DEFAULT_RATE_WAIT));
    }
    (status == StatusCode::TOO_MANY_REQUESTS).then_some(now + DEFAULT_RATE_WAIT)
}

/// The time from the `x-ratelimit-reset` header (epoch seconds).
fn reset_at(headers: &HeaderMap) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(header_number(headers, "x-ratelimit-reset")?, 0)
}

/// A header value as a number.
fn header_number(headers: &HeaderMap, name: &str) -> Option<i64> {
    headers.get(name)?.to_str().ok()?.trim().parse().ok()
}

/// The `message` field of a GitHub error body, else the body itself,
/// cut to 200 characters.
fn error_message(body: &str) -> String {
    let message = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("message")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| body.to_owned());
    message.chars().take(200).collect()
}

#[cfg(test)]
mod tests {
    use reqwest::header::HeaderValue;

    use super::*;
    use crate::domain::test_time;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn rate_limited_until_cases() {
        let now = test_time();
        let reset = now + TimeDelta::seconds(600);
        let reset_text = reset.timestamp().to_string();
        let cases = [
            ("200 is no limit", StatusCode::OK, vec![], None),
            (
                "403 without headers is no limit",
                StatusCode::FORBIDDEN,
                vec![],
                None,
            ),
            (
                "retry-after wins",
                StatusCode::FORBIDDEN,
                vec![("retry-after", "30"), ("x-ratelimit-remaining", "0")],
                Some(now + TimeDelta::seconds(30)),
            ),
            (
                "403 with remaining 0 uses the reset",
                StatusCode::FORBIDDEN,
                vec![
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", reset_text.as_str()),
                ],
                Some(reset),
            ),
            (
                "403 with remaining left is no limit",
                StatusCode::FORBIDDEN,
                vec![
                    ("x-ratelimit-remaining", "12"),
                    ("x-ratelimit-reset", reset_text.as_str()),
                ],
                None,
            ),
            (
                "429 without headers waits the default",
                StatusCode::TOO_MANY_REQUESTS,
                vec![],
                Some(now + DEFAULT_RATE_WAIT),
            ),
        ];
        for (name, status, pairs, expected) in cases {
            assert_eq!(
                rate_limited_until(status, &headers(&pairs), now),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn client_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GithubClient>();
    }

    #[test]
    fn error_message_reads_the_message_field() {
        assert_eq!(
            error_message(r#"{"message":"Not Found","documentation_url":"x"}"#),
            "Not Found"
        );
        assert_eq!(error_message("plain text"), "plain text");
        assert_eq!(error_message(&"x".repeat(300)).len(), 200);
    }
}
