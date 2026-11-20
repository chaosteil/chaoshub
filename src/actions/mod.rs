//! The `Action` trait, the registry and one module per action.
//!
//! An action is one thing that the user can do with a row: merge, review
//! in codeview, send a review and so on. `app` finds the action of a key in `REGISTRY`, asks
//! `applies` and `prepare` on each key press, and routes the result. A
//! new action is one module plus one line in `REGISTRY`.
#![allow(
    dead_code,
    reason = "the concrete actions of phase E2 fill the registry"
)]

mod agent;
mod browser;
mod close;
mod codeview_send;
mod copy;
mod draft;
mod merge;
mod react;
mod reply;
mod rerun_checks;
mod review;

use std::sync::{Arc, LazyLock};

use futures::future::BoxFuture;
use tokio::sync::mpsc::Sender;
use tokio_util::sync::CancellationToken;

use crate::bus::{
    ActionInput, Cmd, ConfirmSpec, Expect, KeyChord, LaunchRequest, Msg, Pending, PendingPart,
    PromptSpec, Selection, Target,
};
use crate::github::GithubClient;
use crate::vcs::lock::RepoLocks;

pub(crate) use codeview_send::STAYS_LOCAL;
pub(crate) use merge::STACK_NOT_LOADED;
pub(crate) use react::ID as REACT_ID;
pub(crate) use reply::ID as REPLY_ID;
pub(crate) use rerun_checks::rerun_plan;

/// What a running action can use. There is no launcher: panes come only
/// from `Prepare::Terminal`.
#[derive(Clone)]
pub(crate) struct Services {
    pub client: Arc<GithubClient>,
    pub locks: RepoLocks,
    /// A child of the root token. Quit and `Cmd::CancelAction` cancel it.
    pub cancel: CancellationToken,
    /// Sends `Msg::ActionEvent` progress, output and popups to `app`.
    pub tx: Sender<Msg>,
}

/// Whether an action applies to a selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Applicability {
    Enabled,
    /// The action does not apply to this kind of row.
    Hidden,
    /// The action applies, but cannot run now. The reason shows in the
    /// status line.
    Disabled(&'static str),
}

/// The group of an action. The actions menu shows a gap row between two
/// groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Group {
    /// The actions that review the pull request.
    Review,
    /// The actions that do not change the pull request.
    Safe,
    /// The actions that change the pull request.
    Change,
}

/// What happens after a key press, before `execute`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Prepare {
    /// Run `execute` at once.
    Run,
    /// Ask the user to confirm, then run `execute` with
    /// `ActionInput::Confirmed`.
    Confirm(ConfirmSpec),
    /// Ask the user for a text, then run `execute` with
    /// `ActionInput::Text`.
    Prompt(PromptSpec),
    /// Run the command in a terminal. `execute` is not called.
    Terminal(LaunchRequest),
    /// Execute this command. `execute` is not called.
    Command(Cmd),
    /// Open the pull request popup of the target on the pane with this
    /// id. `execute` is not called.
    OpenPopup(&'static str),
    /// Open the Review popup of the target. A key of an event there runs
    /// `execute` with `ActionInput::Review`.
    Review,
}

/// One action of the registry.
pub(crate) trait Action: Send + Sync {
    /// The id, for example `merge`. It is also the binding name in
    /// `[keys]` and the table name in `[actions.<id>]`.
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    /// The short label of the hint bar. The default is the id, with
    /// spaces in place of the underscores.
    fn hint(&self) -> String {
        self.id().replace('_', " ")
    }
    /// The short label of the hint bar for `target`. The default is
    /// `hint`.
    fn target_hint(&self, _target: &Target) -> String {
        self.hint()
    }
    /// The label for `target`. The actions menu shows it. The default is
    /// `label`.
    fn target_label(&self, _target: &Target) -> &'static str {
        self.label()
    }
    /// The key of the action when `[keys]` does not bind it.
    fn default_key(&self) -> Option<KeyChord>;
    /// The group of the action in the actions menu.
    fn group(&self) -> Group;
    /// Pure, called on each key press.
    fn applies(&self, sel: &Selection) -> Applicability;
    /// Pure, called on each key press after `applies` gave `Enabled`.
    fn prepare(&self, sel: &Selection) -> Prepare;
    /// The parts of the pull requests that the action changes on GitHub
    /// when it runs with `input`, each with the change that GitHub must
    /// show. `app` pulses each part until the GitHub data shows the change
    /// (see `app::pending`). The default is no part: the action changes
    /// nothing on GitHub.
    fn pending(&self, _sel: &Selection, _input: &ActionInput) -> Vec<Pending> {
        Vec::new()
    }
    /// Runs the action as a task. The runner sends `ActionEvent::Done`
    /// with the result.
    fn execute(
        &self,
        sel: Selection,
        input: ActionInput,
        svc: Services,
    ) -> BoxFuture<'static, Result<String, String>>;
}

/// The action registry, in help order and in menu order. The actions of
/// one group are next to each other.
pub(crate) static REGISTRY: LazyLock<Vec<Box<dyn Action>>> = LazyLock::new(|| {
    vec![
        Box::new(review::Review),
        Box::new(codeview_send::CodeviewSend),
        Box::new(reply::Reply),
        Box::new(react::React),
        Box::new(browser::Browser),
        Box::new(copy::CopyUrl),
        Box::new(copy::CopyStackUrls),
        Box::new(copy::CopyChangeId),
        Box::new(agent::Agent),
        Box::new(merge::MergeAction),
        Box::new(merge::MergeStackAction),
        Box::new(rerun_checks::RerunChecks),
        Box::new(draft::Draft),
        Box::new(close::Close),
    ]
});

/// The part `part` of the pull request of `sel`, which waits for
/// `expect`, for `Action::pending`.
fn own_part(sel: &Selection, part: PendingPart, expect: Expect) -> Vec<Pending> {
    let Target::Pr { pr, .. } = &sel.target;
    vec![Pending {
        pr: pr.key.clone(),
        part,
        expect,
    }]
}

/// The ids and the default keys that occur more than once in `actions`.
/// The key of the reply does not count. It works only in the
/// conversation pane, and the actions menu never shows the reply as an
/// action, so it can have the key of a menu action.
fn duplicates(actions: &[Box<dyn Action>]) -> (Vec<&'static str>, Vec<KeyChord>) {
    let mut ids = Vec::new();
    let mut keys = Vec::new();
    let mut dup_ids = Vec::new();
    let mut dup_keys = Vec::new();
    for action in actions {
        let id = action.id();
        if ids.contains(&id) {
            dup_ids.push(id);
        }
        ids.push(id);
        if let Some(key) = action.default_key().filter(|_| id != REPLY_ID) {
            if keys.contains(&key) {
                dup_keys.push(key);
            }
            keys.push(key);
        }
    }
    (dup_ids, dup_keys)
}

/// Actions for the tests of the registry, the key routing and the
/// runner.
#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use crate::bus::{KeyCode, KeyModifiers};

    /// An action with a fixed key, applicability and preparation. Its
    /// task returns `result`.
    #[derive(Debug, Clone)]
    pub(crate) struct FakeAction {
        pub id: &'static str,
        pub key: Option<KeyChord>,
        pub applies: Applicability,
        pub prepare: Prepare,
        pub result: Result<String, String>,
    }

    impl FakeAction {
        /// An enabled action on the character key `c` that runs at once
        /// and returns `Ok(id)`.
        pub(crate) fn new(id: &'static str, c: char) -> Self {
            Self {
                id,
                key: Some(KeyChord {
                    code: KeyCode::Char(c),
                    mods: KeyModifiers::NONE,
                }),
                applies: Applicability::Enabled,
                prepare: Prepare::Run,
                result: Ok(id.to_owned()),
            }
        }
    }

    impl Action for FakeAction {
        fn id(&self) -> &'static str {
            self.id
        }

        fn label(&self) -> &'static str {
            self.id
        }

        fn default_key(&self) -> Option<KeyChord> {
            self.key
        }

        fn group(&self) -> Group {
            Group::Change
        }

        fn applies(&self, _sel: &Selection) -> Applicability {
            self.applies
        }

        fn prepare(&self, _sel: &Selection) -> Prepare {
            self.prepare.clone()
        }

        fn execute(
            &self,
            _sel: Selection,
            _input: ActionInput,
            _svc: Services,
        ) -> BoxFuture<'static, Result<String, String>> {
            let result = self.result.clone();
            Box::pin(async move { result })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeAction;
    use super::*;

    #[test]
    fn registry_ids_and_keys_are_unique() {
        let (ids, keys) = duplicates(&REGISTRY);
        assert_eq!(ids, [""; 0], "duplicate action ids");
        assert_eq!(keys, [], "duplicate default keys");
    }

    #[test]
    fn the_actions_that_change_github_declare_their_parts() {
        use crate::bus::{ReactSpec, ReplyTo, ReviewKind};
        use crate::domain::detail::ReactionKind;

        let pr: crate::domain::pr::PullRequest =
            serde_json::from_str(include_str!("testdata/pr.json")).unwrap();
        let key = pr.key.clone();
        let sel = Selection {
            target: Target::Pr {
                repo: crate::domain::repo::Repo {
                    key: pr.key.repo.clone(),
                    path: None,
                    settings: None,
                },
                pr,
                stack: Vec::new(),
                local: crate::domain::local::LocalMatch::None,
            },
            me: "me".into(),
            action_config: None,
        };
        let parts = |id: &str, input: ActionInput| -> Vec<PendingPart> {
            let action = REGISTRY.iter().find(|a| a.id() == id).unwrap();
            let parts = action.pending(&sel, &input);
            assert!(parts.iter().all(|p| p.pr == key), "{id}");
            parts.into_iter().map(|p| p.part).collect()
        };
        let review = ActionInput::Review {
            ids: Vec::new(),
            event: ReviewKind::Approve,
            body: String::new(),
        };
        let thread = ActionInput::Reply {
            to: ReplyTo::Thread {
                id: "T_1".into(),
                place: "a.rs:1".into(),
            },
            body: "ok".into(),
        };
        let comment = ActionInput::Reply {
            to: ReplyTo::Pr,
            body: "ok".into(),
        };
        let react = ActionInput::React(ReactSpec {
            subject: "C_1".into(),
            kind: ReactionKind::Heart,
            add: true,
        });
        assert_eq!(parts("codeview_send", review), [PendingPart::Review]);
        assert_eq!(parts("merge", ActionInput::Confirmed), [PendingPart::Merge]);
        assert_eq!(parts("close", ActionInput::Confirmed), [PendingPart::State]);
        assert_eq!(parts("draft", ActionInput::None), [PendingPart::Draft]);
        assert_eq!(
            parts("rerun_checks", ActionInput::None),
            [PendingPart::Checks]
        );
        assert_eq!(parts("reply", thread), [PendingPart::Thread("T_1".into())]);
        assert_eq!(parts("reply", comment), [PendingPart::Comment]);
        assert_eq!(
            parts("react", react),
            [PendingPart::Reaction("C_1".into(), ReactionKind::Heart)]
        );
        for id in [
            "review",
            "browser",
            "copy_url",
            "copy_stack_urls",
            "copy_change_id",
            "agent",
        ] {
            assert_eq!(parts(id, ActionInput::None), [], "{id} changes nothing");
        }
    }

    #[test]
    fn duplicates_finds_repeated_ids_and_keys() {
        let mut no_key = FakeAction::new("c", 'c');
        no_key.key = None;
        let actions: Vec<Box<dyn Action>> = vec![
            Box::new(FakeAction::new("a", 'x')),
            Box::new(FakeAction::new("a", 'y')),
            Box::new(FakeAction::new("b", 'x')),
            Box::new(no_key.clone()),
            Box::new(FakeAction { id: "d", ..no_key }),
            Box::new(FakeAction::new(REPLY_ID, 'x')),
        ];
        let (ids, keys) = duplicates(&actions);
        assert_eq!(ids, ["a"]);
        assert_eq!(keys, [FakeAction::new("b", 'x').key.unwrap()]);
    }
}
