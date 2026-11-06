//! The agent action: opens the pull request popup on the Agent pane.
//!
//! The Agent pane starts the pr-agent runs. pr-agent runs in the local
//! checkout, so the action needs a repo with a path.

use futures::future::BoxFuture;

use crate::actions::{Action, Applicability, Prepare, Services};
use crate::bus::{ActionInput, KeyChord, KeyCode, KeyModifiers, Selection, Target};

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

    /// The popup opens in `app`; the runtime never calls this.
    fn execute(
        &self,
        _sel: Selection,
        _input: ActionInput,
        _svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        Box::pin(async { Err("the agent action opens a pane".to_owned()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::local::LocalMatch;
    use crate::domain::pr::PullRequest;
    use crate::domain::repo::Repo;

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
}
