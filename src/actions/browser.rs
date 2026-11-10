//! The action that opens a pull request in the browser.

use futures::future::BoxFuture;

use crate::actions::{Action, Applicability, Group, Prepare, Services};
use crate::bus::{ActionInput, Cmd, KeyChord, KeyCode, KeyModifiers, Selection, Target};

/// The action that opens the url of a pull request.
pub(crate) struct Browser;

impl Action for Browser {
    fn id(&self) -> &'static str {
        "browser"
    }

    fn label(&self) -> &'static str {
        "open in the browser"
    }

    fn default_key(&self) -> Option<KeyChord> {
        Some(KeyChord {
            code: KeyCode::Char('o'),
            mods: KeyModifiers::NONE,
        })
    }

    fn group(&self) -> Group {
        Group::Safe
    }

    /// A pull request.
    fn applies(&self, _sel: &Selection) -> Applicability {
        Applicability::Enabled
    }

    fn prepare(&self, sel: &Selection) -> Prepare {
        let Target::Pr { pr, .. } = &sel.target;
        Prepare::Command(Cmd::OpenUrl(pr.head.value.url.clone()))
    }

    /// The url opens through `Cmd::OpenUrl`; the runtime never calls
    /// this.
    fn execute(
        &self,
        _sel: Selection,
        _input: ActionInput,
        _svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        Box::pin(async { Err("the browser opens through a command".to_owned()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::local::LocalMatch;
    use crate::domain::pr::PullRequest;
    use crate::domain::repo::Repo;

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

    #[test]
    fn a_pull_request_opens_its_url() {
        let sel = selection();
        assert_eq!(Browser.applies(&sel), Applicability::Enabled);
        assert_eq!(
            Browser.prepare(&sel),
            Prepare::Command(Cmd::OpenUrl(
                "https://github.com/chaosteil/chaoshub/pull/7".into()
            ))
        );
    }
}
