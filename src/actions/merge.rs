//! The merge action.
//!
//! It merges a pull request through the async merge endpoint of GitHub.
//! For a pull request at position k of a stack, GitHub merges the
//! positions 1 to k atomically, so the blockers and the confirm lines
//! cover every entry up to k. The per-repo overrides come from the
//! `[actions.merge.repos."owner/name"]` tables, with the fields of
//! `[merge.repos."owner/name"]`: `method` and `bypass_rules` (off by
//! default).

use crate::bus::{ConfirmSpec, Selection, Target};
use crate::config::schema::{MergeConfig, RepoMerge};
use crate::domain::merge::{MergeMethod, merge_blockers, resolve_method};
use crate::domain::pr::{CiState, PullRequest, ReviewDecision};
use crate::domain::repo::Repo;

use super::{Applicability, Prepare};

/// The id of the merge action.
pub(crate) const ID: &str = "merge";

/// The merge action.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MergeAction;

impl MergeAction {
    /// Hidden for a local-only change. Disabled when the configuration
    /// table is invalid, when the stack entries up to the position are
    /// not loaded, or when an entry has a hard blocker.
    pub(crate) fn applies(sel: &Selection) -> Applicability {
        let Target::Pr { pr, stack, .. } = &sel.target else {
            return Applicability::Hidden;
        };
        if repo_merge(sel).is_err() {
            return Applicability::Disabled("invalid [actions.merge] configuration");
        }
        let Some(entries) = merged_entries(pr, stack) else {
            return Applicability::Disabled("stack not loaded");
        };
        if merge_blockers(&entries).hard.is_empty() {
            Applicability::Enabled
        } else {
            Applicability::Disabled(hard_reason(&entries))
        }
    }

    /// The confirm popup: every merged entry bottom first with its CI and
    /// review state, the merge method, the bypass flag and the warnings.
    pub(crate) fn prepare(sel: &Selection) -> Prepare {
        let Target::Pr {
            pr, stack, repo, ..
        } = &sel.target
        else {
            return Prepare::Run;
        };
        let entries = merged_entries(pr, stack).unwrap_or_else(|| vec![pr.clone()]);
        let settings = repo_merge(sel).unwrap_or_default();
        Prepare::Confirm(ConfirmSpec {
            title: format!("merge #{}?", pr.key.number),
            lines: confirm_lines(&entries, repo, &settings),
            danger: true,
        })
    }
}

/// The merge overrides of the repo of the target, from the
/// `[actions.merge]` table. The defaults without a table or without an
/// entry for the repo. An error when the table does not parse.
pub(crate) fn repo_merge(sel: &Selection) -> Result<RepoMerge, String> {
    let Some(table) = &sel.action_config else {
        return Ok(RepoMerge::default());
    };
    let config: MergeConfig = toml::Value::Table(table.clone())
        .try_into()
        .map_err(|err: toml::de::Error| err.to_string())?;
    Ok(config
        .repos
        .get(sel.target.repo())
        .cloned()
        .unwrap_or_default())
}

/// The merge method of the repo: the override of the configuration, else
/// what the repo permits.
pub(crate) fn method(repo: &Repo, settings: &RepoMerge) -> MergeMethod {
    resolve_method(repo, settings.method.map(MergeMethod::from))
}

/// The pull requests that a merge of `pr` includes: the stack entries
/// with a position up to the position of `pr`, position 1 first, or `pr`
/// alone without a stack. `None` when fewer entries are loaded than the
/// position.
fn merged_entries(pr: &PullRequest, stack: &[PullRequest]) -> Option<Vec<PullRequest>> {
    let Some(stack_ref) = &pr.head.value.stack else {
        return Some(vec![pr.clone()]);
    };
    let top = stack_ref.position;
    let mut entries: Vec<PullRequest> = stack
        .iter()
        .filter(|entry| position(entry).is_some_and(|p| p <= top))
        .cloned()
        .collect();
    entries.sort_by_key(position);
    let wanted = usize::try_from(top).unwrap_or(usize::MAX);
    (entries.len() >= wanted).then_some(entries)
}

/// The stack position of a pull request.
fn position(pr: &PullRequest) -> Option<u32> {
    pr.head.value.stack.as_ref().map(|s| s.position)
}

/// The status line reason for a hard blocker. The reason is static, so
/// it names the kind and not the pull request.
fn hard_reason(entries: &[PullRequest]) -> &'static str {
    let single = entries.len() == 1;
    let draft = entries.iter().any(|pr| pr.head.value.is_draft);
    match (single, draft) {
        (true, true) => "the pull request is a draft",
        (true, false) => "the pull request has conflicts",
        (false, true) => "a pull request of the stack is a draft",
        (false, false) => "a pull request of the stack has conflicts",
    }
}

/// The lines of the confirm popup.
fn confirm_lines(entries: &[PullRequest], repo: &Repo, settings: &RepoMerge) -> Vec<String> {
    let mut lines = Vec::new();
    if entries.len() > 1 {
        lines.push(format!(
            "merges {} pull requests atomically, bottom first:",
            entries.len()
        ));
    }
    for pr in entries {
        let head = &pr.head.value;
        let place = position(pr).map_or_else(String::new, |p| format!("{p} "));
        lines.push(format!(
            "  {place}#{} {}  {}, {}",
            pr.key.number,
            head.title,
            ci_text(pr.ci_state.value),
            review_text(head.review_decision),
        ));
    }
    lines.push(format!("method: {}", method(repo, settings).as_str()));
    if settings.bypass_rules {
        lines.push("bypasses the branch rules".to_owned());
    }
    for warning in merge_blockers(entries).warnings {
        lines.push(format!("warning: {warning}"));
    }
    lines
}

/// The CI state for the confirm popup.
fn ci_text(state: CiState) -> &'static str {
    match state {
        CiState::Success => "CI passed",
        CiState::Failure => "CI failed",
        CiState::Pending => "CI running",
        CiState::None => "no CI",
    }
}

/// The review decision for the confirm popup.
fn review_text(decision: Option<ReviewDecision>) -> &'static str {
    match decision {
        Some(ReviewDecision::Approved) => "approved",
        Some(ReviewDecision::ChangesRequested) => "changes requested",
        Some(ReviewDecision::ReviewRequired) => "review required",
        None => "no review needed",
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::domain::keys::RepoKey;
    use crate::domain::local::{LocalChange, LocalMatch};
    use crate::domain::matching::tests::pr;
    use crate::domain::pr::{MergeStateStatus, Mergeable};
    use crate::domain::repo::RepoSettings;

    fn repo() -> Repo {
        Repo {
            key: RepoKey {
                owner: "o".into(),
                name: "r".into(),
            },
            path: None,
            settings: Some(RepoSettings {
                squash: true,
                merge: true,
                rebase: true,
                default_branch: None,
            }),
        }
    }

    /// A clean, approved pull request with green CI, in the stack `s` at
    /// `position`, or without a stack at position 0.
    fn entry(number: u64, position: u32) -> PullRequest {
        let mut pr = pr(number, if position == 0 { "" } else { "s" }, position);
        pr.head.value.mergeable = Mergeable::Mergeable;
        pr.head.value.merge_state = MergeStateStatus::Clean;
        pr.head.value.review_decision = Some(ReviewDecision::Approved);
        pr.ci_state.value = CiState::Success;
        pr
    }

    fn select(pr: PullRequest, stack: Vec<PullRequest>, config: Option<&str>) -> Selection {
        Selection {
            target: Target::Pr {
                pr,
                stack,
                local: LocalMatch::None,
                repo: repo(),
            },
            me: "me".into(),
            action_config: config.map(|text| toml::from_str(text).unwrap()),
        }
    }

    fn stack3() -> Vec<PullRequest> {
        vec![entry(1, 1), entry(2, 2), entry(3, 3)]
    }

    fn confirm(title: &str, lines: &[&str]) -> Prepare {
        Prepare::Confirm(ConfirmSpec {
            title: title.into(),
            lines: lines.iter().map(|line| (*line).to_owned()).collect(),
            danger: true,
        })
    }

    #[test]
    fn enabled_cases_open_the_confirm_popup() {
        let mut warned = stack3();
        warned[0].head.value.merge_state = MergeStateStatus::Blocked;
        warned[1].head.value.mergeable = Mergeable::Unknown;
        warned[1].ci_state.value = CiState::Pending;
        warned[1].head.value.review_decision = Some(ReviewDecision::ReviewRequired);
        let mut draft_above = stack3();
        draft_above[2].head.value.is_draft = true;
        let override_config = "[repos.\"o/r\"]\nmethod = \"rebase\"\nbypass_rules = true\n";
        let other_repo = "[repos.\"o/other\"]\nmethod = \"merge\"\n";
        let single = ["  #7 Change 7  CI passed, approved", "method: squash"];
        let cases = [
            (
                "single pull request",
                select(entry(7, 0), Vec::new(), None),
                confirm("merge #7?", &single),
            ),
            (
                "stack top merges every entry, bottom first",
                select(
                    entry(3, 3),
                    vec![entry(3, 3), entry(1, 1), entry(2, 2)],
                    None,
                ),
                confirm(
                    "merge #3?",
                    &[
                        "merges 3 pull requests atomically, bottom first:",
                        "  1 #1 Change 1  CI passed, approved",
                        "  2 #2 Change 2  CI passed, approved",
                        "  3 #3 Change 3  CI passed, approved",
                        "method: squash",
                    ],
                ),
            ),
            (
                "stack middle ignores the entries above it",
                select(entry(2, 2), draft_above, None),
                confirm(
                    "merge #2?",
                    &[
                        "merges 2 pull requests atomically, bottom first:",
                        "  1 #1 Change 1  CI passed, approved",
                        "  2 #2 Change 2  CI passed, approved",
                        "method: squash",
                    ],
                ),
            ),
            (
                "stack bottom is one pull request",
                select(entry(1, 1), stack3(), None),
                confirm(
                    "merge #1?",
                    &["  1 #1 Change 1  CI passed, approved", "method: squash"],
                ),
            ),
            (
                "warnings show in the popup",
                select(entry(2, 2), warned, None),
                confirm(
                    "merge #2?",
                    &[
                        "merges 2 pull requests atomically, bottom first:",
                        "  1 #1 Change 1  CI passed, approved",
                        "  2 #2 Change 2  CI running, review required",
                        "method: squash",
                        "warning: #1 is blocked by the branch rules",
                        "warning: #2 merge state is not known yet",
                    ],
                ),
            ),
            (
                "the repo override sets the method and the bypass",
                select(entry(7, 0), Vec::new(), Some(override_config)),
                confirm(
                    "merge #7?",
                    &[
                        "  #7 Change 7  CI passed, approved",
                        "method: rebase",
                        "bypasses the branch rules",
                    ],
                ),
            ),
            (
                "an override of another repo does not apply",
                select(entry(7, 0), Vec::new(), Some(other_repo)),
                confirm("merge #7?", &single),
            ),
        ];
        for (name, sel, prepare) in cases {
            assert_eq!(MergeAction::applies(&sel), Applicability::Enabled, "{name}");
            assert_eq!(MergeAction::prepare(&sel), prepare, "{name}");
        }
    }

    #[test]
    fn disabled_cases_give_the_reason() {
        let mut draft_below = stack3();
        draft_below[0].head.value.is_draft = true;
        let mut conflict = entry(7, 0);
        conflict.head.value.mergeable = Mergeable::Conflicting;
        let mut draft = entry(7, 0);
        draft.head.value.is_draft = true;
        let mut conflict_below = stack3();
        conflict_below[1].head.value.mergeable = Mergeable::Conflicting;
        let cases = [
            (
                "stack not loaded",
                select(entry(3, 3), vec![entry(1, 1), entry(3, 3)], None),
                "stack not loaded",
            ),
            (
                "draft below in the stack",
                select(entry(3, 3), draft_below, None),
                "a pull request of the stack is a draft",
            ),
            (
                "conflict below in the stack",
                select(entry(3, 3), conflict_below, None),
                "a pull request of the stack has conflicts",
            ),
            (
                "draft pull request",
                select(draft, Vec::new(), None),
                "the pull request is a draft",
            ),
            (
                "conflicting pull request",
                select(conflict, Vec::new(), None),
                "the pull request has conflicts",
            ),
            (
                "invalid configuration",
                select(entry(7, 0), Vec::new(), Some("repos = 3")),
                "invalid [actions.merge] configuration",
            ),
        ];
        for (name, sel, reason) in cases {
            assert_eq!(
                MergeAction::applies(&sel),
                Applicability::Disabled(reason),
                "{name}"
            );
        }
    }

    #[test]
    fn a_local_change_hides_the_action() {
        let sel = Selection {
            target: Target::Local {
                change: LocalChange {
                    workspace: "default".into(),
                    change_id: "kxyz".into(),
                    commit_id: "abc".into(),
                    bookmarks: Vec::new(),
                    empty: false,
                    parents: Vec::new(),
                    description: "local".into(),
                },
                repo: repo(),
            },
            me: "me".into(),
            action_config: None,
        };
        assert_eq!(MergeAction::applies(&sel), Applicability::Hidden);
    }
}
