//! The review action: opens codeview.
//!
//! The action opens nvim with `:Codeview pr <N>` in the repo directory, in
//! a pane beside the TUI. `CHAOSHUB_AGENT_ROOT` tells codeview where the
//! pr-agent result files are. `CODEVIEW_PR` holds the pull request data
//! that chaoshub has, so codeview shows the review without a wait for
//! `gh pr view` and checks the data in the background. An older codeview
//! ignores the variable. nvim quits when the codeview session closes:
//! `Session:close()` in codeview sends `User CodeViewSessionClosed`, also
//! for a session of `:Codeview pr`.

use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::actions::{Action, Applicability, Group, Prepare, Services};
use crate::agent::store::Store;
use crate::bus::{ActionInput, KeyChord, KeyCode, KeyModifiers, LaunchRequest, Selection, Target};
use crate::domain::pr::{PrState, PullRequest};

/// The nvim binary that runs codeview.
pub(crate) const NVIM: &str = "nvim";

/// The env variable of codeview that holds the pull request data.
pub(crate) const PR_DATA_VAR: &str = "CODEVIEW_PR";

/// The longest pull request data in bytes. Linux refuses an argument or
/// an env entry of more than 128 KiB, and Rex gets the env as an
/// argument of `env`. A longer body leaves the data out, and codeview
/// reads the pull request with gh.
const PR_DATA_MAX: usize = 100 * 1024;

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

    fn group(&self) -> Group {
        Group::Review
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
            env: agent_env().into_iter().chain(pr_data_env(pr)).collect(),
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

/// The `CODEVIEW_PR` entry with the data of `pr`, in the form of
/// `gh pr view --json`. `None` without a base commit, because codeview
/// then fetches the base and the review opens slower than with gh, and
/// `None` for data above `PR_DATA_MAX`.
fn pr_data_env(pr: &PullRequest) -> Option<(String, String)> {
    let head = &pr.head.value;
    if head.base_oid.is_empty() {
        return None;
    }
    let state = match head.state {
        PrState::Open => "OPEN",
        PrState::Closed => "CLOSED",
        PrState::Merged => "MERGED",
    };
    let head_repo = if head.head_repo.is_empty() {
        Value::Null
    } else {
        json!({ "nameWithOwner": head.head_repo })
    };
    let data = json!({
        "number": pr.key.number,
        "title": head.title,
        "state": state,
        "isDraft": head.is_draft,
        "url": head.url,
        "author": { "login": head.author },
        "headRefName": head.head_ref,
        "baseRefName": head.base_ref,
        "headRefOid": head.head_oid,
        "baseRefOid": head.base_oid,
        "headRepository": head_repo,
        "isCrossRepository": head.cross_repo,
        "body": head.body,
    })
    .to_string();
    (data.len() <= PR_DATA_MAX).then(|| (PR_DATA_VAR.to_owned(), data))
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

    /// The value of the env entry `name` of `req`.
    fn env_value<'a>(req: &'a LaunchRequest, name: &str) -> Option<&'a str> {
        req.env
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// The selection of the fixture with the fields that the data needs.
    fn selection_with_data() -> Selection {
        let mut sel = selection(Some("/code/chaoshub"));
        let Target::Pr { pr, .. } = &mut sel.target;
        let head = &mut pr.head.value;
        head.body = "Line one\r\nLine \"two\"".into();
        head.base_oid = "base7".into();
        head.head_repo = "octo/chaoshub".into();
        head.cross_repo = true;
        sel
    }

    #[test]
    fn prepare_gives_codeview_the_pull_request_data() {
        let req = request(&selection_with_data());
        let data: Value = serde_json::from_str(env_value(&req, PR_DATA_VAR).unwrap()).unwrap();
        assert_eq!(
            data,
            json!({
                "number": 7,
                "title": "Add the review action",
                "state": "OPEN",
                "isDraft": false,
                "url": "https://github.com/chaosteil/chaoshub/pull/7",
                "author": { "login": "chaosteil" },
                "headRefName": "chaosteil/7",
                "baseRefName": "main",
                "headRefOid": "oid7",
                "baseRefOid": "base7",
                "headRepository": { "nameWithOwner": "octo/chaoshub" },
                "isCrossRepository": true,
                "body": "Line one\r\nLine \"two\"",
            })
        );
        assert!(
            env_value(&req, "CHAOSHUB_AGENT_ROOT").is_some(),
            "{:?}",
            req.env
        );
    }

    #[test]
    fn a_pull_request_without_a_head_repo_gives_a_null_head_repository() {
        let mut sel = selection_with_data();
        let Target::Pr { pr, .. } = &mut sel.target;
        pr.head.value.head_repo = String::new();
        let req = request(&sel);
        let data: Value = serde_json::from_str(env_value(&req, PR_DATA_VAR).unwrap()).unwrap();
        assert_eq!(data["headRepository"], Value::Null);
    }

    #[test]
    fn prepare_leaves_out_the_data_without_a_base_commit_or_with_a_long_body() {
        // The fixture comes from a cache without the base commit.
        let req = request(&selection(Some("/code/chaoshub")));
        assert_eq!(env_value(&req, PR_DATA_VAR), None);

        let mut sel = selection_with_data();
        let Target::Pr { pr, .. } = &mut sel.target;
        pr.head.value.body = "x".repeat(PR_DATA_MAX);
        assert_eq!(env_value(&request(&sel), PR_DATA_VAR), None);
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
