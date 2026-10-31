//! The review action: opens codeview.
//!
//! The action opens nvim with `:Codeview pr <N>` in the repo directory, in
//! a pane beside the TUI. nvim quits when the codeview session closes:
//! `Session:close()` in codeview sends `User CodeViewSessionClosed`, also
//! for a session of `:Codeview pr`.

use futures::future::BoxFuture;

use crate::actions::{Action, Applicability, Prepare, Services};
use crate::bus::{ActionInput, KeyChord, KeyCode, KeyModifiers, LaunchRequest, Selection, Target};

/// The action that reviews a pull request in codeview.
pub(crate) struct Review;

impl Review {
    /// The `[actions.review]` key that zooms the pane. `run` fills it
    /// from `[codeview] zoom`.
    const ZOOM: &'static str = "zoom";
}

impl Action for Review {
    fn id(&self) -> &'static str {
        "review"
    }

    fn label(&self) -> &'static str {
        "review in codeview"
    }

    fn default_key(&self) -> Option<KeyChord> {
        Some(KeyChord {
            code: KeyCode::Char('v'),
            mods: KeyModifiers::NONE,
        })
    }

    fn hint(&self) -> crate::actions::Hint {
        crate::actions::Hint::Primary
    }

    /// A pull request of a repo with a local path. A local-only change
    /// has no pull request to review.
    fn applies(&self, sel: &Selection) -> Applicability {
        match &sel.target {
            Target::Pr { repo, .. } if repo.path.is_some() => Applicability::Enabled,
            Target::Pr { .. } => Applicability::Disabled("the repo has no local path"),
            Target::Local { .. } => Applicability::Hidden,
        }
    }

    /// nvim with codeview on the pull request, in the repo directory.
    fn prepare(&self, sel: &Selection) -> Prepare {
        let Target::Pr { pr, repo, .. } = &sel.target else {
            unreachable!("applies hides the review of a local change")
        };
        let number = pr.key.number;
        let zoom = sel
            .action_config
            .as_ref()
            .and_then(|table| table.get(Self::ZOOM))
            .and_then(toml::Value::as_bool)
            .unwrap_or(true);
        Prepare::Terminal(LaunchRequest {
            argv: vec![
                "nvim".to_owned(),
                "-c".to_owned(),
                "autocmd User CodeViewSessionClosed ++once qall".to_owned(),
                "-c".to_owned(),
                format!("Codeview pr {number}"),
            ],
            cwd: repo.path.clone().unwrap_or_default(),
            env: Vec::new(),
            label: format!(
                "chaoshub-review-{}-{}-{number}-{:04x}",
                repo.key.owner,
                repo.key.name,
                rand::random::<u16>()
            ),
            zoom,
        })
    }

    /// The review runs in a terminal; the runtime never calls this.
    fn execute(
        &self,
        _sel: Selection,
        _input: ActionInput,
        _svc: Services,
    ) -> BoxFuture<'static, Result<String, String>> {
        Box::pin(async { Err("the review runs in a terminal".to_owned()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::local::{LocalChange, LocalMatch};
    use crate::domain::pr::PullRequest;
    use crate::domain::repo::Repo;

    /// The selection of the fixture pull request #7 of
    /// `chaosteil/chaoshub`, in a repo at `path`.
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

    fn request(sel: &Selection) -> LaunchRequest {
        match Review.prepare(sel) {
            Prepare::Terminal(request) => request,
            other => panic!("not a terminal: {other:?}"),
        }
    }

    #[test]
    fn applies_needs_a_pull_request_with_a_repo_path() {
        assert_eq!(
            Review.applies(&selection(Some("/code/chaoshub"))),
            Applicability::Enabled
        );
        assert_eq!(
            Review.applies(&selection(None)),
            Applicability::Disabled("the repo has no local path")
        );
        let mut local = selection(Some("/code/chaoshub"));
        local.target = Target::Local {
            change: LocalChange {
                workspace: "default".into(),
                change_id: "zzmqxkrtzzmq".into(),
                commit_id: "c0ffee".into(),
                bookmarks: Vec::new(),
                empty: false,
                parents: Vec::new(),
                description: "wip".into(),
            },
            repo: Repo {
                key: local.target.repo().clone(),
                path: Some("/code/chaoshub".into()),
                settings: None,
            },
        };
        assert_eq!(Review.applies(&local), Applicability::Hidden);
    }

    #[test]
    fn prepare_opens_codeview_on_the_pull_request_in_the_repo() {
        let req = request(&selection(Some("/code/chaoshub")));
        assert_eq!(
            req.argv,
            [
                "nvim",
                "-c",
                "autocmd User CodeViewSessionClosed ++once qall",
                "-c",
                "Codeview pr 7",
            ]
        );
        assert_eq!(req.cwd, std::path::PathBuf::from("/code/chaoshub"));
        assert_eq!(req.env, []);
        assert!(req.zoom, "the pane zooms by default");
        let suffix = req
            .label
            .strip_prefix("chaoshub-review-chaosteil-chaoshub-7-")
            .unwrap();
        assert_eq!(suffix.len(), 4, "{}", req.label);
        assert!(
            suffix.chars().all(|c| c.is_ascii_hexdigit()),
            "{}",
            req.label
        );
    }

    #[test]
    fn the_action_table_turns_the_zoom_off() {
        let mut sel = selection(Some("/code/chaoshub"));
        sel.action_config = Some(toml::from_str("zoom = false").unwrap());
        assert!(!request(&sel).zoom);
        sel.action_config = Some(toml::from_str("zoom = true").unwrap());
        assert!(request(&sel).zoom);
    }
}
