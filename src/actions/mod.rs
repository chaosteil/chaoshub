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
mod rerun_checks;
mod review;

use std::sync::{Arc, LazyLock};

use futures::future::BoxFuture;
use tokio::sync::mpsc::Sender;
use tokio_util::sync::CancellationToken;

use crate::bus::{
    ActionInput, Cmd, ConfirmSpec, KeyChord, LaunchRequest, Msg, PromptSpec, Selection, Target,
};
use crate::github::GithubClient;
use crate::vcs::lock::RepoLocks;

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
    /// The label for `target`. The actions menu shows it. The default is
    /// `label`.
    fn target_label(&self, _target: &Target) -> &'static str {
        self.label()
    }
    /// The key of the action when `[keys]` does not bind it.
    fn default_key(&self) -> Option<KeyChord>;
    /// Pure, called on each key press.
    fn applies(&self, sel: &Selection) -> Applicability;
    /// Pure, called on each key press after `applies` gave `Enabled`.
    fn prepare(&self, sel: &Selection) -> Prepare;
    /// Runs the action as a task. The runner sends `ActionEvent::Done`
    /// with the result.
    fn execute(
        &self,
        sel: Selection,
        input: ActionInput,
        svc: Services,
    ) -> BoxFuture<'static, Result<String, String>>;
}

/// The action registry, in help order.
pub(crate) static REGISTRY: LazyLock<Vec<Box<dyn Action>>> = LazyLock::new(|| {
    vec![
        Box::new(review::Review),
        Box::new(merge::MergeAction),
        Box::new(merge::MergeStackAction),
        Box::new(rerun_checks::RerunChecks),
        Box::new(draft::Draft),
        Box::new(close::Close),
        Box::new(browser::Browser),
        Box::new(copy::CopyUrl),
        Box::new(copy::CopyChangeId),
        Box::new(agent::Agent),
        Box::new(codeview_send::CodeviewSend),
    ]
});

/// The ids and the default keys that occur more than once in `actions`.
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
        if let Some(key) = action.default_key() {
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
    fn duplicates_finds_repeated_ids_and_keys() {
        let mut no_key = FakeAction::new("c", 'c');
        no_key.key = None;
        let actions: Vec<Box<dyn Action>> = vec![
            Box::new(FakeAction::new("a", 'x')),
            Box::new(FakeAction::new("a", 'y')),
            Box::new(FakeAction::new("b", 'x')),
            Box::new(no_key.clone()),
            Box::new(FakeAction { id: "d", ..no_key }),
        ];
        let (ids, keys) = duplicates(&actions);
        assert_eq!(ids, ["a"]);
        assert_eq!(keys, [FakeAction::new("b", 'x').key.unwrap()]);
    }
}
