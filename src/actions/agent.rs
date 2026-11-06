//! The agent action: opens the pull request popup on the Agent pane.
//!
//! The Agent pane starts the pr-agent runs. pr-agent runs in the local
//! checkout, so the action needs a repo with a path. The confirm popup of
//! the `p` key in the Agent pane runs this action: `execute` sets the
//! title and the body of the pull request from `describe.json`.

use futures::future::BoxFuture;

use crate::actions::{Action, Applicability, Prepare, Services};
use crate::agent::store::Store;
use crate::bus::{ActionInput, KeyChord, KeyCode, KeyModifiers, Selection, Target};
use crate::domain::agent::{ToolKind, describe_body, describe_title};
use crate::domain::pr::PullRequest;
use crate::github::GithubClient;

/// The id of the Agent pane.
const PANE: &str = "agent";

/// The action that opens the Agent pane of a pull request.
pub(crate) struct Agent;

impl Action for Agent {
    fn id(&self) -> &'static str {
        "agent"
    }

    fn label(&self) -> &'static str {
        "pr-agent"
    }

    fn default_key(&self) -> Option<KeyChord> {
        Some(KeyChord {
            code: KeyCode::Char('a'),
            mods: KeyModifiers::NONE,
        })
    }

    /// A pull request of a repo with a local path.
    fn applies(&self, sel: &Selection) -> Applicability {
        match &sel.target {
            Target::Pr { repo, .. } if repo.path.is_some() => Applicability::Enabled,
            Target::Pr { .. } => Applicability::Disabled("the repo has no local path"),
        }
    }

    fn prepare(&self, _sel: &Selection) -> Prepare {
        Prepare::OpenPopup(PANE)
    }

    /// Runs after the confirm of the describe result: `updatePullRequest`
    /// with the title and the body of `describe.json`. The follow-up of
    /// the action then refreshes the pull request.
    fn execute(
        &self,
        sel: Selection,
        _input: ActionInput,
        svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        Box::pin(async move {
            let Target::Pr { pr, .. } = sel.target;
            let state_dir = crate::config::paths::state_dir()
                .ok_or_else(|| "no state directory for the results".to_owned())?;
            apply_describe(&svc.client, &Store::new(state_dir), &pr).await
        })
    }
}

/// Sets the title and the body of `pr` from its describe result in
/// `store`.
async fn apply_describe(
    client: &GithubClient,
    store: &Store,
    pr: &PullRequest,
) -> Result<String, String> {
    let record = store
        .read(&pr.key, ToolKind::Describe)
        .map_err(|err| format!("cannot read describe.json: {err}"))?
        .ok_or_else(|| "no describe result".to_owned())?;
    let title = describe_title(&record.data)
        .ok_or_else(|| "the describe result has no title".to_owned())?;
    let body = describe_body(&record.data);
    client
        .update_pr(&pr.node_id, &title, &body)
        .await
        .map_err(|err| err.to_string())?;
    Ok(format!("applied the description to #{}", pr.key.number))
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::domain::agent::{Record, VERSION};
    use crate::domain::local::LocalMatch;
    use crate::domain::pr::PullRequest;
    use crate::domain::repo::Repo;
    use crate::github::TokenSource;

    fn selection(path: Option<&str>) -> Selection {
        let pr: PullRequest = serde_json::from_str(include_str!("testdata/pr.json")).unwrap();
        Selection {
            target: Target::Pr {
                repo: Repo {
                    key: pr.key.repo.clone(),
                    path: path.map(Into::into),
                    settings: None,
                },
                pr,
                stack: Vec::new(),
                local: LocalMatch::None,
            },
            me: "chaosteil".into(),
            action_config: None,
        }
    }

    #[test]
    fn it_needs_a_repo_path_and_opens_the_agent_pane() {
        assert_eq!(
            Agent.applies(&selection(None)),
            Applicability::Disabled("the repo has no local path")
        );
        let sel = selection(Some("/code/chaoshub"));
        assert_eq!(Agent.applies(&sel), Applicability::Enabled);
        assert_eq!(Agent.prepare(&sel), Prepare::OpenPopup("agent"));
    }

    #[tokio::test]
    async fn apply_describe_sends_the_title_and_the_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-ratelimit-remaining", "4990")
                    .insert_header("x-ratelimit-reset", "1790000000")
                    .set_body_json(json!({ "data": {
                        "updatePullRequest": { "pullRequest": { "id": "PR_kwDO7", "title": "New" } }
                    } })),
            )
            .expect(1)
            .mount(&server)
            .await;
        let client =
            GithubClient::new(TokenSource::command("echo", &["tok"]), &server.uri()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().to_path_buf());
        let Target::Pr { pr, .. } = selection(Some("/code")).target;
        let err = apply_describe(&client, &store, &pr).await.unwrap_err();
        assert_eq!(err, "no describe result");
        let record = Record {
            version: VERSION,
            tool: "describe".into(),
            pr: pr.key.number,
            head_sha: "abc".into(),
            model: "m".into(),
            started_at: crate::domain::test_time(),
            finished_at: crate::domain::test_time(),
            usage: json!({}),
            data: json!({"title": "New", "description": "- One", "pr_files": []}),
        };
        store.write(&pr.key, ToolKind::Describe, &record).unwrap();
        let result = apply_describe(&client, &store, &pr).await;
        assert_eq!(
            result,
            Ok(format!("applied the description to #{}", pr.key.number))
        );
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["operationName"], json!("UpdatePr"));
        assert_eq!(
            body["variables"],
            json!({ "id": pr.node_id, "title": "New", "body": "- One" })
        );
    }
}
