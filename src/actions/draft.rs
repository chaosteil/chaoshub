//! The action that toggles draft and ready.

use futures::future::BoxFuture;

use crate::actions::{Action, Applicability, Group, Prepare, Services};
use crate::bus::{ActionInput, KeyChord, KeyCode, KeyModifiers, Selection, Target};
use crate::domain::pr::PrState;

/// The action that converts an open pull request to a draft, or marks a
/// draft as ready for review.
pub(crate) struct Draft;

impl Action for Draft {
    fn id(&self) -> &'static str {
        "draft"
    }

    fn label(&self) -> &'static str {
        "toggle draft / ready"
    }

    fn default_key(&self) -> Option<KeyChord> {
        Some(KeyChord {
            code: KeyCode::Char('d'),
            mods: KeyModifiers::NONE,
        })
    }

    fn group(&self) -> Group {
        Group::Change
    }

    /// An open pull request.
    fn applies(&self, sel: &Selection) -> Applicability {
        match &sel.target {
            Target::Pr { pr, .. } if pr.head.value.state == PrState::Open => Applicability::Enabled,
            Target::Pr { .. } => Applicability::Disabled("the pull request is not open"),
        }
    }

    fn prepare(&self, _sel: &Selection) -> Prepare {
        Prepare::Run
    }

    /// `markPullRequestReadyForReview` for a draft, else
    /// `convertPullRequestToDraft`.
    fn execute(
        &self,
        sel: Selection,
        _input: ActionInput,
        svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        Box::pin(async move {
            let Target::Pr { pr, .. } = sel.target;
            let number = pr.key.number;
            if pr.head.value.is_draft {
                svc.client
                    .mark_ready(&pr.node_id)
                    .await
                    .map_err(|err| err.to_string())?;
                Ok(format!("#{number} is ready for review"))
            } else {
                svc.client
                    .convert_to_draft(&pr.node_id)
                    .await
                    .map_err(|err| err.to_string())?;
                Ok(format!("#{number} is a draft"))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::{Value, json};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::domain::local::LocalMatch;
    use crate::domain::pr::PullRequest;
    use crate::domain::repo::Repo;
    use crate::github::{GithubClient, TokenSource};
    use crate::vcs::lock::RepoLocks;

    fn selection(draft: bool) -> Selection {
        let mut pr: PullRequest = serde_json::from_str(include_str!("testdata/pr.json")).unwrap();
        pr.head.value.is_draft = draft;
        Selection {
            target: Target::Pr {
                repo: Repo {
                    key: pr.key.repo.clone(),
                    path: None,
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

    /// A GraphQL server that answers every request with `data`.
    async fn server(data: Value) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-ratelimit-remaining", "4990")
                    .insert_header("x-ratelimit-reset", "1790000000")
                    .set_body_json(json!({ "data": data })),
            )
            .mount(&server)
            .await;
        server
    }

    fn services(server: &MockServer) -> Services {
        let client = GithubClient::new(TokenSource::command("echo", &["tok"]), &server.uri());
        Services {
            client: Arc::new(client.unwrap()),
            locks: RepoLocks::default(),
            cancel: CancellationToken::new(),
            tx: mpsc::channel(8).0,
        }
    }

    /// The operation name and the variables of each request to `server`.
    async fn sent(server: &MockServer) -> Vec<(Value, Value)> {
        let requests = server.received_requests().await.unwrap();
        requests
            .iter()
            .map(|request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                (body["operationName"].clone(), body["variables"].clone())
            })
            .collect()
    }

    #[test]
    fn applies_to_an_open_pull_request_and_runs_at_once() {
        let sel = selection(false);
        assert_eq!(Draft.applies(&sel), Applicability::Enabled);
        assert_eq!(Draft.prepare(&sel), Prepare::Run);
        let mut merged = selection(false);
        let Target::Pr { pr, .. } = &mut merged.target;
        pr.head.value.state = PrState::Merged;
        assert_eq!(
            Draft.applies(&merged),
            Applicability::Disabled("the pull request is not open")
        );
    }

    #[tokio::test]
    async fn a_ready_pull_request_becomes_a_draft() {
        let server = server(json!({
            "convertPullRequestToDraft": { "pullRequest": { "id": "PR_kwDO7", "isDraft": true } }
        }))
        .await;
        let result = Draft
            .execute(selection(false), ActionInput::None, services(&server))
            .await;
        assert_eq!(result, Ok("#7 is a draft".to_owned()));
        assert_eq!(
            sent(&server).await,
            [(json!("ConvertToDraft"), json!({ "id": "PR_kwDO7" }))]
        );
    }

    #[tokio::test]
    async fn a_draft_becomes_ready_for_review() {
        let server = server(json!({
            "markPullRequestReadyForReview": { "pullRequest": { "id": "PR_kwDO7", "isDraft": false } }
        }))
        .await;
        let result = Draft
            .execute(selection(true), ActionInput::None, services(&server))
            .await;
        assert_eq!(result, Ok("#7 is ready for review".to_owned()));
        assert_eq!(
            sent(&server).await,
            [(json!("MarkReady"), json!({ "id": "PR_kwDO7" }))]
        );
    }

    #[tokio::test]
    async fn a_failed_mutation_is_the_error_text() {
        let server = server(json!({ "convertPullRequestToDraft": null })).await;
        let result = Draft
            .execute(selection(false), ActionInput::None, services(&server))
            .await;
        let err = result.unwrap_err();
        assert!(
            err.contains("convertPullRequestToDraft returned no result"),
            "{err}"
        );
    }
}
