//! The approve action.

use futures::future::BoxFuture;

use crate::actions::{Action, Applicability, Prepare, Services};
use crate::bus::{ActionInput, KeyChord, KeyCode, KeyModifiers, Selection, Target};
use crate::domain::pr::PrState;
use crate::github::ReviewEvent;

/// The action that submits an approving review of a pull request.
pub(crate) struct Approve;

impl Action for Approve {
    fn id(&self) -> &'static str {
        "approve"
    }

    fn label(&self) -> &'static str {
        "approve"
    }

    fn default_key(&self) -> Option<KeyChord> {
        Some(KeyChord {
            code: KeyCode::Char('A'),
            mods: KeyModifiers::NONE,
        })
    }

    fn hint(&self) -> crate::actions::Hint {
        crate::actions::Hint::Primary
    }

    /// An open pull request of another author.
    fn applies(&self, sel: &Selection) -> Applicability {
        match &sel.target {
            Target::Pr { pr, .. } if pr.head.value.state != PrState::Open => {
                Applicability::Disabled("the pull request is not open")
            }
            Target::Pr { pr, .. } if pr.head.value.author == sel.me => {
                Applicability::Disabled("you cannot review your own pull request")
            }
            Target::Pr { .. } => Applicability::Enabled,
        }
    }

    fn prepare(&self, _sel: &Selection) -> Prepare {
        Prepare::Run
    }

    /// `addPullRequestReview` with the event `APPROVE`.
    fn execute(
        &self,
        sel: Selection,
        _input: ActionInput,
        svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        Box::pin(async move {
            let Target::Pr { pr, .. } = sel.target;
            svc.client
                .add_review(&pr.node_id, ReviewEvent::Approve, None)
                .await
                .map_err(|err| err.to_string())?;
            Ok(format!("approved #{}", pr.key.number))
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

    fn selection() -> Selection {
        let pr: PullRequest = serde_json::from_str(include_str!("testdata/pr.json")).unwrap();
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
            me: "reviewer".into(),
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

    #[test]
    fn applies_to_an_open_pull_request_and_runs_at_once() {
        let sel = selection();
        assert_eq!(Approve.applies(&sel), Applicability::Enabled);
        assert_eq!(Approve.prepare(&sel), Prepare::Run);
        let mut merged = selection();
        let Target::Pr { pr, .. } = &mut merged.target;
        pr.head.value.state = PrState::Merged;
        assert_eq!(
            Approve.applies(&merged),
            Applicability::Disabled("the pull request is not open")
        );
    }

    #[test]
    fn your_own_pull_request_is_disabled() {
        let mut own = selection();
        own.me = "chaosteil".into();
        assert_eq!(
            Approve.applies(&own),
            Applicability::Disabled("you cannot review your own pull request")
        );
    }

    #[tokio::test]
    async fn execute_sends_an_approving_review() {
        let server = server(json!({
            "addPullRequestReview": { "pullRequestReview": { "id": "PRR_1", "state": "APPROVED" } }
        }))
        .await;
        let result = Approve
            .execute(selection(), ActionInput::None, services(&server))
            .await;
        assert_eq!(result, Ok("approved #7".to_owned()));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["operationName"], json!("AddReview"));
        assert_eq!(
            body["variables"],
            json!({ "id": "PR_kwDO7", "event": "APPROVE", "body": null })
        );
    }

    #[tokio::test]
    async fn a_failed_mutation_is_the_error_text() {
        let server = server(json!({ "addPullRequestReview": null })).await;
        let result = Approve
            .execute(selection(), ActionInput::None, services(&server))
            .await;
        let err = result.unwrap_err();
        assert!(
            err.contains("addPullRequestReview returned no result"),
            "{err}"
        );
    }
}
