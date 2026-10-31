//! REST calls: merge-async, merge status, check re-runs and stacks.

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::bus::MergeProgress;
use crate::domain::keys::PrKey;
use crate::github::{GithubClient, GithubError, error_message, status_error};

/// The REST API version of the merge-async endpoints.
const MERGE_API_VERSION: &str = "2026-03-10";
/// The media type of the GitHub REST API.
const ACCEPT: &str = "application/vnd.github+json";

/// The arguments of an async merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MergeArgs<'a> {
    /// `merge`, `squash` or `rebase`.
    pub method: &'a str,
    pub commit_title: &'a str,
    pub commit_message: &'a str,
    /// The head commit that the pull request must still have.
    pub sha: &'a str,
    /// Bypass the repository rules that the user may bypass.
    pub bypass_rules: bool,
}

/// The body of `PUT .../merge-async`.
#[derive(Debug, Serialize)]
struct MergeBody<'a> {
    merge_method: &'a str,
    commit_title: &'a str,
    commit_message: &'a str,
    sha: &'a str,
    merge_action: &'a str,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    bypass_rules: bool,
}

/// The response of both merge-async endpoints.
#[derive(Debug, Deserialize)]
struct MergeJson {
    status: String,
    #[serde(default)]
    details: MergeDetailsJson,
}

/// The `details` of a merge-async response. Its fields depend on the
/// status.
#[derive(Debug, Default, Deserialize)]
struct MergeDetailsJson {
    message: Option<String>,
    uuid: Option<String>,
    sha: Option<String>,
}

/// The answer to a merge request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MergeStarted {
    /// The id to poll with [`GithubClient::merge_async_status`]. `None`
    /// when the result is final at once.
    pub uuid: Option<String>,
    pub progress: MergeProgress,
}

impl GithubClient {
    /// Requests an async merge of the pull request `key`. A 202 starts a
    /// merge, a 409 returns the merge that runs already, a 200 is final
    /// (merged or enqueued), and a 400 (closed, draft) is a failed
    /// progress with the GitHub message. For a stacked pull request the
    /// merge includes every open entry below it.
    pub(crate) async fn merge_async(
        &self,
        key: &PrKey,
        args: &MergeArgs<'_>,
    ) -> Result<MergeStarted, GithubError> {
        let url = self.url(&format!(
            "/repos/{}/{}/pulls/{}/merge-async",
            key.repo.owner, key.repo.name, key.number
        ));
        let body = MergeBody {
            merge_method: args.method,
            commit_title: args.commit_title,
            commit_message: args.commit_message,
            sha: args.sha,
            merge_action: "default",
            bypass_rules: args.bypass_rules,
        };
        let response = self
            .send_raw(|http| {
                http.put(&url)
                    .header("accept", ACCEPT)
                    .header("x-github-api-version", MERGE_API_VERSION)
                    .json(&body)
            })
            .await?;
        let status = response.status();
        match status {
            StatusCode::OK | StatusCode::ACCEPTED | StatusCode::CONFLICT => {
                let json: MergeJson = response.json().await?;
                tracing::info!(pr = key.number, %status, merge_status = %json.status, "merge-async");
                let uuid = json.details.uuid.clone();
                Ok(MergeStarted {
                    uuid,
                    progress: progress(json)?,
                })
            }
            StatusCode::BAD_REQUEST => {
                let message = error_message(&response.text().await.unwrap_or_default());
                tracing::info!(pr = key.number, %message, "merge-async refused");
                Ok(MergeStarted {
                    uuid: None,
                    progress: MergeProgress::Failed(message),
                })
            }
            _ => Err(status_error(response).await),
        }
    }

    /// The current state of the async merge `uuid` of the pull request
    /// `key`. GitHub keeps the result for 24 hours; then it is a 404.
    pub(crate) async fn merge_async_status(
        &self,
        key: &PrKey,
        uuid: &str,
    ) -> Result<MergeProgress, GithubError> {
        let url = self.url(&format!(
            "/repos/{}/{}/pulls/{}/merge-async/{uuid}",
            key.repo.owner, key.repo.name, key.number
        ));
        let response = self
            .send(|http| {
                http.get(&url)
                    .header("accept", ACCEPT)
                    .header("x-github-api-version", MERGE_API_VERSION)
            })
            .await?;
        progress(response.json().await?)
    }
}

/// The merge progress of a merge-async response.
fn progress(json: MergeJson) -> Result<MergeProgress, GithubError> {
    match json.status.as_str() {
        "pending" => Ok(MergeProgress::Pending),
        "merged" => Ok(MergeProgress::Merged {
            sha: json.details.sha.unwrap_or_default(),
        }),
        "enqueued" => Ok(MergeProgress::Enqueued),
        "failed" => Ok(MergeProgress::Failed(
            json.details
                .message
                .unwrap_or_else(|| "merge failed".to_owned()),
        )),
        other => Err(GithubError::Parse(format!("unknown merge status: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::domain::keys::RepoKey;
    use crate::github::graphql::tests::client;

    fn key() -> PrKey {
        PrKey {
            repo: RepoKey {
                owner: "acme".into(),
                name: "deploy".into(),
            },
            number: 42,
        }
    }

    fn args() -> MergeArgs<'static> {
        MergeArgs {
            method: "squash",
            commit_title: "Add the thing (#42)",
            commit_message: "Body",
            sha: "abc123",
            bypass_rules: false,
        }
    }

    fn pending(uuid: &str) -> Value {
        json!({
            "status": "pending",
            "details": {
                "message": "Merge request accepted",
                "uuid": uuid,
                "merge_method": "squash",
                "merge_action": "default",
                "expected_head_sha": "abc123",
                "bypass_rules": false
            }
        })
    }

    #[tokio::test]
    async fn merge_202_then_merged() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/acme/deploy/pulls/42/merge-async"))
            .and(header("x-github-api-version", "2026-03-10"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(ResponseTemplate::new(202).set_body_json(pending("u-1")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/deploy/pulls/42/merge-async/u-1"))
            .and(header("x-github-api-version", "2026-03-10"))
            .respond_with(ResponseTemplate::new(200).set_body_json(pending("u-1")))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/deploy/pulls/42/merge-async/u-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "status": "merged",
                "details": { "message": "Pull request merged", "sha": "def456" }
            })))
            .mount(&server)
            .await;
        let client = client(&server);

        let started = client.merge_async(&key(), &args()).await.unwrap();
        assert_eq!(
            started,
            MergeStarted {
                uuid: Some("u-1".into()),
                progress: MergeProgress::Pending
            }
        );
        let uuid = started.uuid.unwrap();
        assert_eq!(
            client.merge_async_status(&key(), &uuid).await.unwrap(),
            MergeProgress::Pending
        );
        assert_eq!(
            client.merge_async_status(&key(), &uuid).await.unwrap(),
            MergeProgress::Merged {
                sha: "def456".into()
            }
        );

        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(
            body,
            json!({
                "merge_method": "squash",
                "commit_title": "Add the thing (#42)",
                "commit_message": "Body",
                "sha": "abc123",
                "merge_action": "default"
            })
        );
    }

    #[tokio::test]
    async fn merge_sends_bypass_rules_when_set() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .respond_with(ResponseTemplate::new(202).set_body_json(pending("u-1")))
            .mount(&server)
            .await;
        let args = MergeArgs {
            bypass_rules: true,
            ..args()
        };
        client(&server).merge_async(&key(), &args).await.unwrap();
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["bypass_rules"], json!(true));
    }

    #[tokio::test]
    async fn merge_answers_that_end_at_once() {
        let cases = [
            (
                "200 enqueued",
                200,
                json!({ "status": "enqueued", "details": { "message": "In the merge queue" } }),
                MergeStarted {
                    uuid: None,
                    progress: MergeProgress::Enqueued,
                },
            ),
            (
                "200 merged",
                200,
                json!({ "status": "merged", "details": { "message": "Merged", "sha": "def456" } }),
                MergeStarted {
                    uuid: None,
                    progress: MergeProgress::Merged {
                        sha: "def456".into(),
                    },
                },
            ),
            (
                "409 returns the running merge",
                409,
                pending("u-9"),
                MergeStarted {
                    uuid: Some("u-9".into()),
                    progress: MergeProgress::Pending,
                },
            ),
            (
                "400 is a failed merge",
                400,
                json!({ "message": "Pull request is in draft state" }),
                MergeStarted {
                    uuid: None,
                    progress: MergeProgress::Failed("Pull request is in draft state".into()),
                },
            ),
        ];
        for (name, status, body, expected) in cases {
            let server = MockServer::start().await;
            Mock::given(method("PUT"))
                .respond_with(ResponseTemplate::new(status).set_body_json(body))
                .mount(&server)
                .await;
            let started = client(&server).merge_async(&key(), &args()).await.unwrap();
            assert_eq!(started, expected, "{name}");
        }
    }

    #[tokio::test]
    async fn merge_errors() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .respond_with(
                ResponseTemplate::new(422).set_body_json(json!({ "message": "Validation Failed" })),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(404).set_body_json(json!({ "message": "Not Found" })),
            )
            .mount(&server)
            .await;
        let client = client(&server);
        let err = client.merge_async(&key(), &args()).await.unwrap_err();
        assert!(
            matches!(&err, GithubError::Http { status: Some(422), message } if message == "Validation Failed"),
            "{err:?}"
        );
        let err = client.merge_async_status(&key(), "u-1").await.unwrap_err();
        assert!(
            matches!(
                err,
                GithubError::Http {
                    status: Some(404),
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn progress_cases() {
        let failed = MergeJson {
            status: "failed".into(),
            details: MergeDetailsJson {
                message: Some("Required status check is failing".into()),
                ..MergeDetailsJson::default()
            },
        };
        assert_eq!(
            progress(failed).unwrap(),
            MergeProgress::Failed("Required status check is failing".into())
        );
        let unknown = MergeJson {
            status: "exploded".into(),
            details: MergeDetailsJson::default(),
        };
        assert!(matches!(progress(unknown), Err(GithubError::Parse(_))));
    }
}
