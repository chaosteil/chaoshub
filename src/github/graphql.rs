//! Runs GraphQL queries and reads the rate limit info.

use std::time::Instant;

use chrono::{DateTime, Utc};
use graphql_client::{GraphQLQuery, QueryBody};
use serde::Deserialize;

use crate::bus::RateInfo;
use crate::github::{GithubClient, GithubError, header_number, reset_at};

/// The `rateLimit { cost remaining resetAt }` selection of a query.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RateLimitJson {
    cost: u32,
    remaining: u32,
    reset_at: DateTime<Utc>,
}

/// One entry of the GraphQL `errors` list.
#[derive(Debug, Deserialize)]
struct ErrorJson {
    message: String,
    #[serde(rename = "type")]
    kind: Option<String>,
}

/// A GraphQL response before the typed decode of `data`.
#[derive(Debug, Deserialize)]
struct ResponseJson {
    data: Option<serde_json::Value>,
    #[serde(default)]
    errors: Vec<ErrorJson>,
}

impl GithubClient {
    /// Runs the query `Q` with `vars` against `/graphql`. Returns the typed
    /// data and the rate limit info. The info comes from the `rateLimit`
    /// field of the query, else from the `x-ratelimit-*` headers with a
    /// cost of 0 (mutations cannot select `rateLimit`).
    ///
    /// It logs the time from the send to the end of the parse, as
    /// `elapsed_ms`.
    pub(crate) async fn run<Q: GraphQLQuery>(
        &self,
        vars: Q::Variables,
    ) -> Result<(Q::ResponseData, RateInfo), GithubError> {
        let body = Q::build_query(vars);
        let operation = body.operation_name;
        let start = Instant::now();
        let result = self.exchange::<Q>(&body).await;
        let elapsed_ms = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
        match &result {
            Ok((_, rate)) => tracing::info!(
                operation,
                cost = rate.last_cost,
                remaining = rate.remaining,
                reset_at = %rate.reset_at,
                elapsed_ms,
                "graphql"
            ),
            Err(err) => tracing::warn!(operation, elapsed_ms, %err, "graphql failed"),
        }
        result
    }

    /// Sends the query `body`, then reads the typed data and the rate
    /// limit info of the response.
    async fn exchange<Q: GraphQLQuery>(
        &self,
        body: &QueryBody<Q::Variables>,
    ) -> Result<(Q::ResponseData, RateInfo), GithubError> {
        let operation = body.operation_name;
        let url = self.url("/graphql");
        let response = self.send(|http| http.post(&url).json(body)).await?;
        let headers = response.headers().clone();
        let text = response.text().await?;
        let parsed: ResponseJson = serde_json::from_str(&text)
            .map_err(|err| GithubError::Parse(format!("{operation}: {err}")))?;

        if parsed
            .errors
            .iter()
            .any(|err| err.kind.as_deref() == Some("RATE_LIMITED"))
        {
            let until = reset_at(&headers).unwrap_or_else(|| Utc::now() + super::DEFAULT_RATE_WAIT);
            tracing::warn!(operation, %until, "GitHub GraphQL rate limit");
            return Err(GithubError::RateLimited { until });
        }
        let messages: Vec<String> = parsed.errors.into_iter().map(|err| err.message).collect();
        let Some(mut data) = parsed.data.filter(|data| !data.is_null()) else {
            return Err(if messages.is_empty() {
                GithubError::Parse(format!("{operation}: no data"))
            } else {
                GithubError::Graphql(messages)
            });
        };
        if !messages.is_empty() {
            tracing::warn!(operation, errors = ?messages, "GitHub GraphQL partial errors");
        }

        let rate = match data.get_mut("rateLimit").map(serde_json::Value::take) {
            Some(value) if !value.is_null() => {
                let limit: RateLimitJson = serde_json::from_value(value)
                    .map_err(|err| GithubError::Parse(format!("{operation} rateLimit: {err}")))?;
                RateInfo {
                    remaining: limit.remaining,
                    reset_at: limit.reset_at,
                    last_cost: limit.cost,
                }
            }
            _ => header_rate(&headers)
                .ok_or_else(|| GithubError::Parse(format!("{operation}: no rate limit info")))?,
        };
        let data = serde_json::from_value(data)
            .map_err(|err| GithubError::Parse(format!("{operation}: {err}")))?;
        Ok((data, rate))
    }
}

/// The rate limit info from the `x-ratelimit-*` headers, with a cost of 0.
fn header_rate(headers: &reqwest::header::HeaderMap) -> Option<RateInfo> {
    Some(RateInfo {
        remaining: u32::try_from(header_number(headers, "x-ratelimit-remaining")?).ok()?,
        reset_at: reset_at(headers)?,
        last_cost: 0,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use serde_json::{Value, json};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::github::TokenSource;

    /// A hand-written query for the runner tests.
    pub(crate) struct TestQuery;

    impl GraphQLQuery for TestQuery {
        type Variables = Value;
        type ResponseData = Value;

        fn build_query(variables: Value) -> QueryBody<Value> {
            QueryBody {
                variables,
                query: "query Test { viewer { login } }",
                operation_name: "Test",
            }
        }
    }

    /// A client for `server` with the fixed token `tok`.
    pub(crate) fn client(server: &MockServer) -> GithubClient {
        GithubClient::new(TokenSource::command("echo", &["tok"]), &server.uri()).unwrap()
    }

    /// A `rateLimit` value for test responses.
    pub(crate) fn rate_json() -> Value {
        json!({ "cost": 1, "remaining": 4999, "resetAt": "2026-10-30T12:00:00Z" })
    }

    fn reset() -> DateTime<Utc> {
        "2026-10-30T12:00:00Z".parse().unwrap()
    }

    #[tokio::test]
    async fn ok_returns_data_and_rate() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "viewer": { "login": "chaosteil" }, "rateLimit": rate_json() }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let (data, rate) = client(&server).run::<TestQuery>(json!({})).await.unwrap();
        assert_eq!(data["viewer"]["login"], "chaosteil");
        assert!(data.get("rateLimit").unwrap().is_null());
        assert_eq!(
            rate,
            RateInfo {
                remaining: 4999,
                reset_at: reset(),
                last_cost: 1
            }
        );
    }

    #[tokio::test]
    async fn rate_falls_back_to_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-ratelimit-remaining", "42")
                    .insert_header("x-ratelimit-reset", reset().timestamp().to_string())
                    .set_body_json(json!({ "data": { "closePullRequest": null } })),
            )
            .mount(&server)
            .await;
        let (_, rate) = client(&server).run::<TestQuery>(json!({})).await.unwrap();
        assert_eq!(
            rate,
            RateInfo {
                remaining: 42,
                reset_at: reset(),
                last_cost: 0
            }
        );
    }

    #[tokio::test]
    async fn unauthorized_refreshes_the_token_once() {
        let dir = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        Mock::given(header("authorization", "Bearer tok-1"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(header("authorization", "Bearer tok-2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "viewer": { "login": "chaosteil" }, "rateLimit": rate_json() }
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = GithubClient::new(TokenSource::counting(dir.path()), &server.uri()).unwrap();
        let (data, _) = client.run::<TestQuery>(json!({})).await.unwrap();
        assert_eq!(data["viewer"]["login"], "chaosteil");
    }

    #[tokio::test]
    async fn unauthorized_twice_is_auth() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401))
            .expect(2)
            .mount(&server)
            .await;
        let err = client(&server)
            .run::<TestQuery>(json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, GithubError::Auth), "{err:?}");
    }

    #[tokio::test]
    async fn forbidden_with_no_remaining_is_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(403)
                    .insert_header("x-ratelimit-remaining", "0")
                    .insert_header("x-ratelimit-reset", reset().timestamp().to_string()),
            )
            .mount(&server)
            .await;
        let err = client(&server)
            .run::<TestQuery>(json!({}))
            .await
            .unwrap_err();
        assert!(
            matches!(err, GithubError::RateLimited { until } if until == reset()),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn forbidden_without_limit_is_http() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_json(json!({ "message": "Resource not accessible" })),
            )
            .mount(&server)
            .await;
        let err = client(&server)
            .run::<TestQuery>(json!({}))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, GithubError::Http { status: Some(403), message } if message == "Resource not accessible"),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn too_many_requests_honors_retry_after() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "120"))
            .mount(&server)
            .await;
        let before = Utc::now();
        let err = client(&server)
            .run::<TestQuery>(json!({}))
            .await
            .unwrap_err();
        let GithubError::RateLimited { until } = err else {
            panic!("{err:?}");
        };
        let wait = until - before;
        assert!(
            wait.num_seconds() >= 119 && wait.num_seconds() <= 125,
            "{wait}"
        );
    }

    #[tokio::test]
    async fn graphql_rate_limit_error_is_rate_limited() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-ratelimit-reset", reset().timestamp().to_string())
                    .set_body_json(json!({
                        "errors": [{ "type": "RATE_LIMITED", "message": "API rate limit exceeded" }]
                    })),
            )
            .mount(&server)
            .await;
        let err = client(&server)
            .run::<TestQuery>(json!({}))
            .await
            .unwrap_err();
        assert!(
            matches!(err, GithubError::RateLimited { until } if until == reset()),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn errors_without_data_are_graphql_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": null,
                "errors": [{ "message": "Field 'x' doesn't exist" }]
            })))
            .mount(&server)
            .await;
        let err = client(&server)
            .run::<TestQuery>(json!({}))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, GithubError::Graphql(messages) if messages == &["Field 'x' doesn't exist"]),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn partial_errors_keep_the_data() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": { "nodes": [null], "rateLimit": rate_json() },
                "errors": [{ "type": "NOT_FOUND", "message": "Could not resolve to a node" }]
            })))
            .mount(&server)
            .await;
        let (data, _) = client(&server).run::<TestQuery>(json!({})).await.unwrap();
        assert_eq!(data["nodes"], json!([null]));
    }

    #[tokio::test]
    async fn no_response_is_http_without_status() {
        let client =
            GithubClient::new(TokenSource::command("echo", &["tok"]), "http://127.0.0.1:1")
                .unwrap();
        let err = client.run::<TestQuery>(json!({})).await.unwrap_err();
        assert!(
            matches!(err, GithubError::Http { status: None, .. }),
            "{err:?}"
        );
    }
}
