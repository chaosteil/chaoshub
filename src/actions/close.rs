//! The close action.

use futures::future::BoxFuture;

use crate::actions::{Action, Applicability, Prepare, Services};
use crate::bus::{ActionInput, ConfirmSpec, KeyChord, KeyCode, KeyModifiers, Selection, Target};
use crate::domain::pr::PrState;

/// The action that closes a pull request without a merge, after a
/// confirm.
pub(crate) struct Close;

impl Action for Close {
    fn id(&self) -> &'static str {
        "close"
    }

    fn label(&self) -> &'static str {
        "close"
    }

    fn default_key(&self) -> Option<KeyChord> {
        Some(KeyChord {
            code: KeyCode::Char('c'),
            mods: KeyModifiers::NONE,
        })
    }

    /// An open pull request.
    fn applies(&self, sel: &Selection) -> Applicability {
        match &sel.target {
            Target::Pr { pr, .. } if pr.head.value.state == PrState::Open => Applicability::Enabled,
            Target::Pr { .. } => Applicability::Disabled("the pull request is not open"),
        }
    }

    /// A danger confirm with the number and the title.
    fn prepare(&self, sel: &Selection) -> Prepare {
        let Target::Pr { pr, .. } = &sel.target;
        let number = pr.key.number;
        Prepare::Confirm(ConfirmSpec {
            title: format!("close #{number}?"),
            lines: vec![
                format!("#{number} {}", pr.head.value.title),
                "closes the pull request without a merge".to_owned(),
            ],
            danger: true,
        })
    }

    /// `closePullRequest`.
    fn execute(
        &self,
        sel: Selection,
        _input: ActionInput,
        svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        Box::pin(async move {
            let Target::Pr { pr, .. } = sel.target;
            svc.client
                .close_pr(&pr.node_id)
                .await
                .map_err(|err| err.to_string())?;
            Ok(format!("closed #{}", pr.key.number))
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
    fn applies_to_an_open_pull_request() {
        assert_eq!(Close.applies(&selection()), Applicability::Enabled);
        let mut closed = selection();
        let Target::Pr { pr, .. } = &mut closed.target;
        pr.head.value.state = PrState::Closed;
        assert_eq!(
            Close.applies(&closed),
            Applicability::Disabled("the pull request is not open")
        );
    }

    #[test]
    fn prepare_asks_for_a_danger_confirm() {
        let Prepare::Confirm(spec) = Close.prepare(&selection()) else {
            panic!("not a confirm");
        };
        assert!(spec.danger);
        insta::assert_debug_snapshot!(spec);
    }

    #[tokio::test]
    async fn execute_closes_the_pull_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-ratelimit-remaining", "4990")
                    .insert_header("x-ratelimit-reset", "1790000000")
                    .set_body_json(json!({ "data": {
                        "closePullRequest": { "pullRequest": { "id": "PR_kwDO7", "state": "CLOSED" } }
                    } })),
            )
            .expect(1)
            .mount(&server)
            .await;
        let result = Close
            .execute(selection(), ActionInput::Confirmed, services(&server))
            .await;
        assert_eq!(result, Ok("closed #7".to_owned()));
        let requests = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["operationName"], json!("ClosePr"));
        assert_eq!(body["variables"], json!({ "id": "PR_kwDO7" }));
    }
}
