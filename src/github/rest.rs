//! REST calls: merge-async, merge status, check re-runs and stacks.

use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

use crate::bus::MergeProgress;
use crate::domain::files::{ChangedFile, FileStatus, PrFiles};
use crate::domain::keys::{PrKey, RepoKey};
use crate::github::{GithubClient, GithubError, error_message, status_error};

/// The REST API version of the merge-async endpoints.
const MERGE_API_VERSION: &str = "2026-03-10";
/// The media type of the GitHub REST API.
const ACCEPT: &str = "application/vnd.github+json";
/// The largest page of the pull request files endpoint.
const FILES_PAGE_SIZE: usize = 100;
/// The number of file pages that [`GithubClient::pr_files`] reads.
const FILES_PAGES: u32 = 3;

/// One item of `GET .../pulls/{number}/files`.
#[derive(Debug, Deserialize)]
struct FileJson {
    filename: String,
    status: String,
    additions: u32,
    deletions: u32,
    /// Absent for a binary file or a very large diff.
    patch: Option<String>,
    previous_filename: Option<String>,
}

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

    /// Re-runs the failed jobs of the workflow run `run_id`.
    pub(crate) async fn rerun_failed_jobs(
        &self,
        repo: &RepoKey,
        run_id: u64,
    ) -> Result<(), GithubError> {
        let url = self.url(&format!(
            "/repos/{}/{}/actions/runs/{run_id}/rerun-failed-jobs",
            repo.owner, repo.name
        ));
        self.send(|http| http.post(&url).header("accept", ACCEPT))
            .await?;
        tracing::info!(run_id, "re-run of failed jobs requested");
        Ok(())
    }

    /// Re-requests the check suite `suite_id`, for checks without a
    /// workflow run.
    pub(crate) async fn rerequest_suite(
        &self,
        repo: &RepoKey,
        suite_id: u64,
    ) -> Result<(), GithubError> {
        let url = self.url(&format!(
            "/repos/{}/{}/check-suites/{suite_id}/rerequest",
            repo.owner, repo.name
        ));
        self.send(|http| http.post(&url).header("accept", ACCEPT))
            .await?;
        tracing::info!(suite_id, "check suite re-requested");
        Ok(())
    }

    /// The changed files of the pull request `key` at the head
    /// `head_oid`. It reads at most 3 pages of 100 files. When the third
    /// page is full, the result is `truncated`.
    pub(crate) async fn pr_files(
        &self,
        key: &PrKey,
        head_oid: &str,
    ) -> Result<PrFiles, GithubError> {
        let mut files = Vec::new();
        let mut truncated = false;
        for page in 1..=FILES_PAGES {
            let url = self.url(&format!(
                "/repos/{}/{}/pulls/{}/files?per_page={FILES_PAGE_SIZE}&page={page}",
                key.repo.owner, key.repo.name, key.number
            ));
            let response = self
                .send(|http| http.get(&url).header("accept", ACCEPT))
                .await?;
            let items: Vec<FileJson> = response.json().await?;
            let full = items.len() >= FILES_PAGE_SIZE;
            files.extend(items.into_iter().map(changed_file));
            if !full {
                break;
            }
            if page == FILES_PAGES {
                truncated = true;
            }
        }
        Ok(PrFiles {
            files,
            truncated,
            head_oid: head_oid.to_owned(),
        })
    }
}

/// A changed file from one item of the files endpoint. The terminal
/// renderer does not expand tabs, so each tab of the patch becomes 4
/// spaces.
fn changed_file(json: FileJson) -> ChangedFile {
    ChangedFile {
        path: json.filename,
        previous_path: json.previous_filename,
        status: match json.status.as_str() {
            "added" => FileStatus::Added,
            "removed" => FileStatus::Removed,
            "modified" => FileStatus::Modified,
            "renamed" => FileStatus::Renamed,
            "copied" => FileStatus::Copied,
            "unchanged" => FileStatus::Unchanged,
            _ => FileStatus::Changed,
        },
        additions: json.additions,
        deletions: json.deletions,
        patch: json.patch.map(|patch| patch.replace('\t', "    ")),
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
    use wiremock::matchers::{header, method, path, query_param};
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

    #[tokio::test]
    async fn rerun_calls_post_to_the_run_and_the_suite() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/acme/deploy/actions/runs/7/rerun-failed-jobs"))
            .and(header("authorization", "Bearer tok"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/repos/acme/deploy/check-suites/9/rerequest"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        let client = client(&server);
        client.rerun_failed_jobs(&key().repo, 7).await.unwrap();
        client.rerequest_suite(&key().repo, 9).await.unwrap();
    }

    #[tokio::test]
    async fn a_refused_rerun_is_an_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_json(json!({ "message": "This workflow run is not completed" })),
            )
            .mount(&server)
            .await;
        let err = client(&server)
            .rerun_failed_jobs(&key().repo, 7)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, GithubError::Http { status: Some(403), message } if message == "This workflow run is not completed"),
            "{err:?}"
        );
    }

    /// A page of `count` modified files.
    fn full_page(count: usize) -> Value {
        Value::Array(
            (0..count)
                .map(|n| {
                    json!({
                        "filename": format!("src/f{n}.rs"),
                        "status": "modified",
                        "additions": 1,
                        "deletions": 0,
                        "patch": "@@ -0,0 +1 @@\n+x"
                    })
                })
                .collect(),
        )
    }

    /// Answers the files page `page` of #42 with `body`.
    async fn mount_files_page(server: &MockServer, page: &str, body: Value) {
        Mock::given(method("GET"))
            .and(path("/repos/acme/deploy/pulls/42/files"))
            .and(query_param("per_page", "100"))
            .and(query_param("page", page))
            .and(header("accept", "application/vnd.github+json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn pr_files_reads_one_page() {
        let server = MockServer::start().await;
        let body: Value = serde_json::from_str(include_str!("testdata/pr_files.json")).unwrap();
        mount_files_page(&server, "1", body).await;

        let files = client(&server).pr_files(&key(), "abc123").await.unwrap();

        assert!(!files.truncated);
        assert_eq!(files.head_oid, "abc123");
        let summary: Vec<_> = files
            .files
            .iter()
            .map(|f| {
                (
                    f.path.as_str(),
                    f.previous_path.as_deref(),
                    f.status,
                    f.additions,
                    f.deletions,
                    f.patch.is_some(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("docs/usage.md", None, FileStatus::Added, 3, 0, true),
                ("old.txt", None, FileStatus::Removed, 0, 2, true),
                (
                    "src/app/panes/files.rs",
                    None,
                    FileStatus::Modified,
                    2,
                    1,
                    true
                ),
                (
                    "src/main.rs",
                    Some("src/start.rs"),
                    FileStatus::Renamed,
                    1,
                    1,
                    true
                ),
                ("assets/logo.png", None, FileStatus::Modified, 0, 0, false),
            ]
        );
        assert_eq!(
            files.files[2].patch.as_deref(),
            Some(
                "@@ -1,3 +1,4 @@\n fn main() {\n-    println!(\"old\");\n+    println!(\"new\");\n+    println!(\"more\");\n }\n\\ No newline at end of file"
            )
        );
    }

    #[tokio::test]
    async fn pr_files_stops_after_a_short_page() {
        let server = MockServer::start().await;
        mount_files_page(&server, "1", full_page(100)).await;
        mount_files_page(&server, "2", full_page(7)).await;

        let files = client(&server).pr_files(&key(), "abc123").await.unwrap();

        assert_eq!(files.files.len(), 107);
        assert!(!files.truncated);
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn pr_files_is_truncated_after_three_full_pages() {
        let server = MockServer::start().await;
        for page in ["1", "2", "3"] {
            mount_files_page(&server, page, full_page(100)).await;
        }

        let files = client(&server).pr_files(&key(), "abc123").await.unwrap();

        assert_eq!(files.files.len(), 300);
        assert!(files.truncated);
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
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
