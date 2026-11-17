//! The reply action: sends a reply from the conversation pane.
//!
//! The reply key of the conversation pane opens a text box for the
//! comment of the cursor. The text box runs this action with
//! `ActionInput::Reply`. A reply in a review thread goes into the
//! thread. A reply to an issue comment or to a review is a new comment
//! on the pull request. A list row has no comment, so the action is
//! hidden there and in the actions menu of the pull request.

use futures::future::BoxFuture;

use crate::actions::{Action, Applicability, Group, Prepare, Services, own_part};
use crate::bus::{
    ActionInput, Expect, KeyChord, KeyCode, KeyModifiers, Pending, PendingPart, ReplyTo, Selection,
    Target,
};

/// The id of the reply action. It is also the binding name of the reply
/// key in the conversation pane.
pub(crate) const ID: &str = "reply";

/// The id of the conversation pane.
const PANE: &str = "conversation";

/// The action that sends a reply in the conversation of a pull request.
pub(crate) struct Reply;

impl Action for Reply {
    fn id(&self) -> &'static str {
        ID
    }

    fn label(&self) -> &'static str {
        "reply to a comment"
    }

    fn default_key(&self) -> Option<KeyChord> {
        Some(KeyChord {
            code: KeyCode::Char('R'),
            mods: KeyModifiers::NONE,
        })
    }

    fn group(&self) -> Group {
        Group::Review
    }

    /// Hidden: a reply needs a comment, and only the conversation pane
    /// knows the comment of the cursor.
    fn applies(&self, _sel: &Selection) -> Applicability {
        Applicability::Hidden
    }

    /// The conversation pane. The runtime does not call this, because
    /// the action is hidden.
    fn prepare(&self, _sel: &Selection) -> Prepare {
        Prepare::OpenPopup(PANE)
    }

    /// The thread of the reply, or a new comment on the pull request.
    fn pending(&self, sel: &Selection, input: &ActionInput) -> Vec<Pending> {
        let part = match input {
            ActionInput::Reply {
                to: ReplyTo::Thread { id, .. },
                ..
            } => PendingPart::Thread(id.clone()),
            ActionInput::Reply {
                to: ReplyTo::Pr, ..
            } => PendingPart::Comment,
            _ => return Vec::new(),
        };
        own_part(sel, part, Expect::NewComment)
    }

    /// `addPullRequestReviewThreadReply` for a thread, else `addComment`
    /// on the pull request.
    fn execute(
        &self,
        sel: Selection,
        input: ActionInput,
        svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        Box::pin(async move {
            let ActionInput::Reply { to, body } = input else {
                return Err("no reply text".to_owned());
            };
            let Target::Pr { pr, .. } = sel.target;
            match to {
                ReplyTo::Thread { id, place } => {
                    svc.client
                        .add_thread_reply(&id, &body)
                        .await
                        .map_err(|err| err.to_string())?;
                    Ok(format!("replied on {place}"))
                }
                ReplyTo::Pr => {
                    svc.client
                        .add_comment(&pr.node_id, &body)
                        .await
                        .map_err(|err| err.to_string())?;
                    Ok(format!("commented on #{}", pr.key.number))
                }
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

    /// A GraphQL server that answers every request with `body`.
    async fn server(body: Value) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/graphql"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-ratelimit-remaining", "4990")
                    .insert_header("x-ratelimit-reset", "1790000000")
                    .set_body_json(body),
            )
            .mount(&server)
            .await;
        server
    }

    /// The operation name and the variables of the requests to `server`.
    async fn sent(server: &MockServer) -> Vec<(Value, Value)> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                (body["operationName"].clone(), body["variables"].clone())
            })
            .collect()
    }

    fn reply(to: ReplyTo, body: &str) -> ActionInput {
        ActionInput::Reply {
            to,
            body: body.to_owned(),
        }
    }

    #[test]
    fn the_action_is_hidden_outside_the_conversation_pane() {
        assert_eq!(Reply.applies(&selection()), Applicability::Hidden);
        assert_eq!(
            Reply.prepare(&selection()),
            Prepare::OpenPopup("conversation")
        );
    }

    #[tokio::test]
    async fn a_reply_in_a_thread_goes_to_the_thread() {
        let server = server(json!({ "data": {
            "addPullRequestReviewThreadReply": { "comment": { "id": "PRRC_1" } }
        } }))
        .await;
        let to = ReplyTo::Thread {
            id: "PRRT_42".into(),
            place: "src/ui/list.rs:42".into(),
        };
        let result = Reply
            .execute(selection(), reply(to, "Fixed."), services(&server))
            .await;
        assert_eq!(result, Ok("replied on src/ui/list.rs:42".to_owned()));
        assert_eq!(
            sent(&server).await,
            [(
                json!("AddThreadReply"),
                json!({ "id": "PRRT_42", "body": "Fixed." })
            )]
        );
    }

    #[tokio::test]
    async fn a_reply_to_a_comment_is_a_comment_on_the_pull_request() {
        let server = server(json!({ "data": {
            "addComment": { "commentEdge": { "node": { "id": "IC_1" } } }
        } }))
        .await;
        let body = "> @alice: Why?\n\nBecause.";
        let result = Reply
            .execute(selection(), reply(ReplyTo::Pr, body), services(&server))
            .await;
        assert_eq!(result, Ok("commented on #7".to_owned()));
        assert_eq!(
            sent(&server).await,
            [(
                json!("AddComment"),
                json!({ "id": "PR_kwDO7", "body": body })
            )]
        );
    }

    #[tokio::test]
    async fn a_graphql_error_is_the_result() {
        let server = server(json!({
            "data": null,
            "errors": [{ "message": "Could not resolve to a node with the global id of 'PRRT_42'" }]
        }))
        .await;
        let to = ReplyTo::Thread {
            id: "PRRT_42".into(),
            place: "src/ui/list.rs:42".into(),
        };
        let err = Reply
            .execute(selection(), reply(to, "Fixed."), services(&server))
            .await
            .unwrap_err();
        assert!(err.contains("Could not resolve"), "{err}");
    }

    #[tokio::test]
    async fn an_input_without_a_reply_sends_nothing() {
        let server = server(json!({ "data": {} })).await;
        let result = Reply
            .execute(selection(), ActionInput::None, services(&server))
            .await;
        assert_eq!(result, Err("no reply text".to_owned()));
        assert_eq!(sent(&server).await, []);
    }
}
