//! The react action: adds or removes a reaction of the viewer from the
//! conversation pane.
//!
//! Enter on a comment of the conversation pane opens the comment menu.
//! A reaction of the menu runs this action with `ActionInput::React`. A
//! list row has no comment, so the action is hidden there and in the
//! actions menu of the pull request.

use futures::future::BoxFuture;

use crate::actions::{Action, Applicability, Group, Prepare, Services, own_part};
use crate::bus::{
    ActionInput, Expect, KeyChord, Pending, PendingPart, ReactSpec, Selection, Target,
};

/// The id of the react action.
pub(crate) const ID: &str = "react";

/// The id of the conversation pane.
const PANE: &str = "conversation";

/// The action that adds or removes a reaction of the viewer on a comment
/// or a review.
pub(crate) struct React;

impl Action for React {
    fn id(&self) -> &'static str {
        ID
    }

    fn label(&self) -> &'static str {
        "react"
    }

    /// No key: the comment menu of the conversation pane has the keys of
    /// the reactions.
    fn default_key(&self) -> Option<KeyChord> {
        None
    }

    /// Safe: a reaction does not change the pull request in the list,
    /// so the PR view does not send `pr_changed` for it.
    fn group(&self) -> Group {
        Group::Safe
    }

    /// Hidden: a reaction needs a comment, and only the conversation
    /// pane knows the comment of the cursor.
    fn applies(&self, _sel: &Selection) -> Applicability {
        Applicability::Hidden
    }

    /// The conversation pane. The runtime does not call this, because
    /// the action is hidden.
    fn prepare(&self, _sel: &Selection) -> Prepare {
        Prepare::OpenPopup(PANE)
    }

    /// The reaction on its comment or review, which the viewer then has
    /// or does not have.
    fn pending(&self, sel: &Selection, input: &ActionInput) -> Vec<Pending> {
        let ActionInput::React(spec) = input else {
            return Vec::new();
        };
        let part = PendingPart::Reaction(spec.subject.clone(), spec.kind);
        own_part(sel, part, Expect::Reaction(spec.add))
    }

    /// `addReaction` to add the reaction, else `removeReaction`.
    fn execute(
        &self,
        sel: Selection,
        input: ActionInput,
        svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        Box::pin(async move {
            let ActionInput::React(ReactSpec { subject, kind, add }) = input else {
                return Err("no reaction".to_owned());
            };
            let Target::Pr { pr, .. } = sel.target;
            let number = pr.key.number;
            let emoji = kind.emoji();
            if add {
                svc.client
                    .add_reaction(&subject, kind)
                    .await
                    .map_err(|err| err.to_string())?;
                Ok(format!("reacted {emoji} on #{number}"))
            } else {
                svc.client
                    .remove_reaction(&subject, kind)
                    .await
                    .map_err(|err| err.to_string())?;
                Ok(format!("removed {emoji} on #{number}"))
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
    use crate::domain::detail::ReactionKind;
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

    fn react(subject: &str, kind: ReactionKind, add: bool) -> ActionInput {
        ActionInput::React(ReactSpec {
            subject: subject.to_owned(),
            kind,
            add,
        })
    }

    #[test]
    fn the_action_is_hidden_and_has_no_key() {
        assert_eq!(React.applies(&selection()), Applicability::Hidden);
        assert_eq!(
            React.prepare(&selection()),
            Prepare::OpenPopup("conversation")
        );
        assert_eq!(React.default_key(), None);
    }

    #[tokio::test]
    async fn an_add_and_a_remove_send_their_mutation() {
        let server = server(json!({ "data": {
            "addReaction": { "subject": { "__typename": "IssueComment", "id": "IC_1" } },
            "removeReaction": { "subject": { "__typename": "PullRequestReview", "id": "PRR_2" } }
        } }))
        .await;
        let result = React
            .execute(
                selection(),
                react("IC_1", ReactionKind::Hooray, true),
                services(&server),
            )
            .await;
        assert_eq!(result, Ok("reacted 🎉 on #7".to_owned()));
        let result = React
            .execute(
                selection(),
                react("PRR_2", ReactionKind::ThumbsUp, false),
                services(&server),
            )
            .await;
        assert_eq!(result, Ok("removed 👍 on #7".to_owned()));
        assert_eq!(
            sent(&server).await,
            [
                (
                    json!("AddReaction"),
                    json!({ "id": "IC_1", "content": "HOORAY" })
                ),
                (
                    json!("RemoveReaction"),
                    json!({ "id": "PRR_2", "content": "THUMBS_UP" })
                ),
            ]
        );
    }

    #[tokio::test]
    async fn a_graphql_error_is_the_result() {
        let server = server(json!({
            "data": null,
            "errors": [{ "message": "Could not resolve to a node with the global id of 'IC_1'" }]
        }))
        .await;
        let err = React
            .execute(
                selection(),
                react("IC_1", ReactionKind::Eyes, true),
                services(&server),
            )
            .await
            .unwrap_err();
        assert!(err.contains("Could not resolve"), "{err}");
    }

    #[tokio::test]
    async fn an_input_without_a_reaction_sends_nothing() {
        let server = server(json!({ "data": {} })).await;
        let result = React
            .execute(selection(), ActionInput::None, services(&server))
            .await;
        assert_eq!(result, Err("no reaction".to_owned()));
        assert_eq!(sent(&server).await, []);
    }
}
