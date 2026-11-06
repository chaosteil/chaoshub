//! The review action: opens codeview.
//!
//! The action opens nvim with `:Codeview pr <N>` in the repo directory, in
//! a pane beside the TUI. `CHAOSHUB_AGENT_ROOT` tells codeview where the
//! pr-agent result files are. nvim quits when the codeview session closes:
//! `Session:close()` in codeview sends `User CodeViewSessionClosed`, also
//! for a session of `:Codeview pr`.

use futures::future::BoxFuture;

use crate::actions::{Action, Applicability, Prepare, Services};
use crate::agent::store::Store;
use crate::bus::{ActionInput, KeyChord, KeyCode, KeyModifiers, LaunchRequest, Selection, Target};

/// The nvim binary that runs codeview.
pub(crate) const NVIM: &str = "nvim";

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

    /// A pull request of a repo with a local path.
    fn applies(&self, sel: &Selection) -> Applicability {
        match &sel.target {
            Target::Pr { repo, .. } if repo.path.is_some() => Applicability::Enabled,
            Target::Pr { .. } => Applicability::Disabled("the repo has no local path"),
        }
    }

    /// nvim with codeview on the pull request, in the repo directory.
    fn prepare(&self, sel: &Selection) -> Prepare {
        let Target::Pr { pr, repo, .. } = &sel.target;
        let number = pr.key.number;
        let zoom = sel
            .action_config
            .as_ref()
            .and_then(|table| table.get(Self::ZOOM))
            .and_then(toml::Value::as_bool)
            .unwrap_or(true);
        Prepare::Terminal(LaunchRequest {
            argv: vec![
                NVIM.to_owned(),
                "-c".to_owned(),
                "autocmd User CodeViewSessionClosed ++once qall".to_owned(),
                "-c".to_owned(),
                format!("Codeview pr {number}"),
            ],
            cwd: repo.path.clone().unwrap_or_default(),
            env: agent_env(),
            label: format!(
                "chaoshub-review-{}-{}-{number}-{:04x}",
                repo.key.owner,
                repo.key.name,
                rand::random::<u16>()
            ),
            zoom,
            split: None,
            ratio: None,
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

/// The env that points codeview to the pr-agent results:
/// `CHAOSHUB_AGENT_ROOT=<state_dir>/agent`. Empty without a state
/// directory.
pub(crate) fn agent_env() -> Vec<(String, String)> {
    crate::config::paths::state_dir()
        .map(|dir| {
            let root = Store::new(dir).root();
            vec![(
                "CHAOSHUB_AGENT_ROOT".to_owned(),
                root.to_string_lossy().into_owned(),
            )]
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::local::LocalMatch;
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
        let root = crate::config::paths::state_dir().unwrap().join("agent");
        assert_eq!(
            req.env,
            [(
                "CHAOSHUB_AGENT_ROOT".to_owned(),
                root.to_string_lossy().into_owned()
            )]
        );
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
