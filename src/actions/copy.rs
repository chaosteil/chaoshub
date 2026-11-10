//! The actions that copy the url or the change id.

use futures::future::BoxFuture;

use crate::actions::{Action, Applicability, Prepare, Services};
use crate::bus::{ActionInput, Cmd, KeyChord, KeyCode, KeyModifiers, Selection, Target};
use crate::domain::local::LocalMatch;

/// The action that copies the url of a pull request.
pub(crate) struct CopyUrl;

/// The action that copies the jj change id of a pull request.
pub(crate) struct CopyChangeId;

/// A key without modifiers.
fn key(c: char) -> KeyChord {
    KeyChord {
        code: KeyCode::Char(c),
        mods: KeyModifiers::NONE,
    }
}

/// The change id of the local change that matches the pull request of the
/// target. `None` for a pull request without a local change.
fn change_id(target: &Target) -> Option<&str> {
    let Target::Pr { local, .. } = target;
    match local {
        LocalMatch::Synced { change_id, .. } | LocalMatch::Diverged { change_id, .. } => {
            Some(change_id)
        }
        LocalMatch::None => None,
    }
}

impl Action for CopyUrl {
    fn id(&self) -> &'static str {
        "copy_url"
    }

    fn label(&self) -> &'static str {
        "copy the url"
    }

    fn default_key(&self) -> Option<KeyChord> {
        Some(key('y'))
    }

    /// A pull request.
    fn applies(&self, _sel: &Selection) -> Applicability {
        Applicability::Enabled
    }

    fn prepare(&self, sel: &Selection) -> Prepare {
        let Target::Pr { pr, .. } = &sel.target;
        Prepare::Command(Cmd::Copy(pr.head.value.url.clone()))
    }

    /// The url goes to the clipboard through `Cmd::Copy`; the runtime
    /// never calls this.
    fn execute(
        &self,
        _sel: Selection,
        _input: ActionInput,
        _svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        Box::pin(async { Err("the copy runs through a command".to_owned()) })
    }
}

impl Action for CopyChangeId {
    fn id(&self) -> &'static str {
        "copy_change_id"
    }

    fn label(&self) -> &'static str {
        "copy the change id"
    }

    fn default_key(&self) -> Option<KeyChord> {
        Some(key('Y'))
    }

    /// A pull request with a local change.
    fn applies(&self, sel: &Selection) -> Applicability {
        match change_id(&sel.target) {
            Some(_) => Applicability::Enabled,
            None => Applicability::Hidden,
        }
    }

    fn prepare(&self, sel: &Selection) -> Prepare {
        let id = change_id(&sel.target).unwrap_or_default();
        Prepare::Command(Cmd::Copy(id.to_owned()))
    }

    /// The change id goes to the clipboard through `Cmd::Copy`; the
    /// runtime never calls this.
    fn execute(
        &self,
        _sel: Selection,
        _input: ActionInput,
        _svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        Box::pin(async { Err("the copy runs through a command".to_owned()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::local::Vcs;
    use crate::domain::pr::PullRequest;
    use crate::domain::repo::Repo;

    fn selection(local: LocalMatch) -> Selection {
        let pr: PullRequest = serde_json::from_str(include_str!("testdata/pr.json")).unwrap();
        Selection {
            target: Target::Pr {
                repo: Repo {
                    key: pr.key.repo.clone(),
                    path: Some("/code/chaoshub".into()),
                    settings: None,
                },
                pr,
                stack: Vec::new(),
                local,
            },
            me: "chaosteil".into(),
            action_config: None,
        }
    }

    #[test]
    fn y_copies_the_url_of_a_pull_request() {
        let sel = selection(LocalMatch::None);
        assert_eq!(CopyUrl.applies(&sel), Applicability::Enabled);
        assert_eq!(
            CopyUrl.prepare(&sel),
            Prepare::Command(Cmd::Copy(
                "https://github.com/chaosteil/chaoshub/pull/7".into()
            ))
        );
    }

    #[test]
    fn shift_y_copies_the_change_id_of_a_match() {
        let copy = |sel: &Selection| (CopyChangeId.applies(sel), CopyChangeId.prepare(sel));
        let enabled = |id: &str| {
            (
                Applicability::Enabled,
                Prepare::Command(Cmd::Copy(id.to_owned())),
            )
        };
        let synced = selection(LocalMatch::Synced {
            workspace: "default".into(),
            change_id: "kpqvxwyz".into(),
            commit_id: "oid7".into(),
            vcs: Vcs::Jj,
        });
        assert_eq!(copy(&synced), enabled("kpqvxwyz"));
        let diverged = selection(LocalMatch::Diverged {
            workspace: "agent".into(),
            change_id: "mnoplqrs".into(),
            commit_id: "c0ffee".into(),
            vcs: Vcs::Jj,
        });
        assert_eq!(copy(&diverged), enabled("mnoplqrs"));
        assert_eq!(
            CopyChangeId.applies(&selection(LocalMatch::None)),
            Applicability::Hidden
        );
    }
}
