//! The request changes action.

use futures::future::BoxFuture;

use crate::actions::{Action, Applicability, Prepare, Services};
use crate::bus::{ActionInput, KeyChord, KeyCode, KeyModifiers, PromptSpec, Selection, Target};
use crate::domain::pr::PrState;
use crate::github::ReviewEvent;

/// The action that asks for a text and submits a review that requests
/// changes, with the text as the body.
pub(crate) struct RequestChanges;

impl Action for RequestChanges {
    fn id(&self) -> &'static str {
        "request_changes"
    }

    fn label(&self) -> &'static str {
        "request changes"
    }

    fn default_key(&self) -> Option<KeyChord> {
        Some(KeyChord {
            code: KeyCode::Char('R'),
            mods: KeyModifiers::NONE,
        })
    }

    /// An open pull request.
    fn applies(&self, sel: &Selection) -> Applicability {
        match &sel.target {
            Target::Pr { pr, .. } if pr.head.value.state == PrState::Open => Applicability::Enabled,
            Target::Pr { .. } => Applicability::Disabled("the pull request is not open"),
            Target::Local { .. } => Applicability::Hidden,
        }
    }

    /// A prompt for the review body.
    fn prepare(&self, sel: &Selection) -> Prepare {
        let Target::Pr { pr, .. } = &sel.target else {
            unreachable!("applies hides the review of a local change")
        };
        Prepare::Prompt(PromptSpec {
            title: format!("request changes on #{}", pr.key.number),
            placeholder: "what to change".to_owned(),
        })
    }

    /// `addPullRequestReview` with the event `REQUEST_CHANGES` and the
    /// prompt text as the body.
    fn execute(
        &self,
        sel: Selection,
        input: ActionInput,
        svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        Box::pin(async move {
            let Target::Pr { pr, .. } = sel.target else {
                return Err("a local change has no review".to_owned());
            };
            let body = match input {
                ActionInput::Text(text) => text,
                ActionInput::None | ActionInput::Confirmed => String::new(),
            };
            svc.client
                .add_review(&pr.node_id, ReviewEvent::RequestChanges, Some(&body))
                .await
                .map_err(|err| err.to_string())?;
            Ok(format!("requested changes on #{}", pr.key.number))
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

    #[test]
    fn applies_to_an_open_pull_request_and_asks_for_a_text() {
        let sel = selection();
        assert_eq!(RequestChanges.applies(&sel), Applicability::Enabled);
        assert_eq!(
            RequestChanges.prepare(&sel),
            Prepare::Prompt(PromptSpec {
                title: "request changes on #7".to_owned(),
                placeholder: "what to change".to_owned(),
            })
        );
        let mut closed = selection();
        if let Target::Pr { pr, .. } = &mut closed.target {
            pr.head.value.state = PrState::Closed;
        }
        assert_eq!(
            RequestChanges.applies(&closed),
            Applicability::Disabled("the pull request is not open")
        );
    }

    #[tokio::test]
    async fn execute_sends_the_prompt_text_as_the_body() {
        let server = server(json!({
            "addPullRequestReview": { "pullRequestReview": { "id": "PRR_1", "state": "CHANGES_REQUESTED" } }
        }))
        .await;
        let input = ActionInput::Text("Please split the commit.".to_owned());
        let result = RequestChanges
            .execute(selection(), input, services(&server))
            .await;
        assert_eq!(result, Ok("requested changes on #7".to_owned()));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["operationName"], json!("AddReview"));
        assert_eq!(
            body["variables"],
            json!({
                "id": "PR_kwDO7",
                "event": "REQUEST_CHANGES",
                "body": "Please split the commit."
            })
        );
    }

    #[tokio::test]
    async fn a_failed_mutation_is_the_error_text() {
        let server = server(json!({ "addPullRequestReview": null })).await;
        let input = ActionInput::Text("x".to_owned());
        let result = RequestChanges
            .execute(selection(), input, services(&server))
            .await;
        let err = result.unwrap_err();
        assert!(
            err.contains("addPullRequestReview returned no result"),
            "{err}"
        );
    }
}
