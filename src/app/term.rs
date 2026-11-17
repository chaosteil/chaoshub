//! Derives the terminal status and title from the app state.
//!
//! `term_status` is pure. It is part of the `FrameModel`, so it is rebuilt
//! only with the generation. It follows the status table of section 1.14
//! of the plan:
//!
//! | Situation | OSC 7501 | OSC 9;4 |
//! |---|---|---|
//! | Idle | `idle` | off |
//! | Confirm or prompt overlay open | `blocked`, `question`, the overlay title | unchanged |
//! | Merge pending | `working`, `merging #N` | indeterminate |
//! | Merge queued | `working`, `#N in merge queue` | indeterminate |
//! | Merge merged | `done`, `merged #N` | 100 for 2 s, then off |
//! | Merge failed | `error`, the reason | error for 5 s, then off |
//! | First sync or requested sync running | `working`, `syncing <n> repos` (`syncing 1 repo`) | indeterminate |
//! | Prefetch batch of more than one ref | `working`, `prefetching` | the percent done |
//! | Sync failed, worker died | `error`, the reason | error for 5 s, then off |
//! | Rate limited | `error`, `rate limited until HH:MM` | paused |
//! | Review pane open | `idle`: the pane is the user's | unchanged |
//!
//! A failure stays until the next key press or the next successful sync.
//! A final merge state stays until the next key press after the end of
//! the run, when the pull request shows the end.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

use crate::app::frame::{SPINNER_FRAME, spinner_at};
use crate::app::notify::ready_count;
use crate::app::panes::description;
use crate::app::state::{AppState, MergeState, OverlayState};
use crate::bus::{BlockedKind, MergeProgress, Progress, Scope, TermState, TermStatus};
use crate::domain::plural;
use crate::domain::pr::{CiState, PrHead, PrState, PullRequest, ReviewDecision};

/// How long the progress bar shows an error after a failure.
pub(crate) const ERROR_PROGRESS: TimeDelta = TimeDelta::seconds(5);
/// How long the progress bar shows 100 after a merge.
pub(crate) const DONE_PROGRESS: TimeDelta = TimeDelta::seconds(2);

/// The terminal status for the app state. An open confirm or prompt
/// overlay turns the state into `blocked` and keeps the progress.
pub(crate) fn term_status(state: &AppState) -> TermStatus {
    let status = base_status(state);
    match question(state) {
        Some(title) => TermStatus {
            state: TermState::Blocked {
                kind: BlockedKind::Question,
            },
            msg: Some(title),
            progress: status.progress,
        },
        None => status,
    }
}

/// The maximum length of the PR title in the title of the standalone PR
/// view, in chars. Rex tabs are narrow.
const PR_TITLE_MAX: usize = 30;

/// How long one frame of the spinner in the title shows: two frames of
/// the status bar spinner. The title changes only with the frame
/// generation, and the status bar spinner bumps the generation once per
/// `SPINNER_FRAME`. As a result, the title changes at most 5 times per
/// second, and `TermWriter` writes it only on a change.
const TITLE_SPINNER_FRAME: Duration = SPINNER_FRAME.saturating_mul(2);

/// The title of the list or the standalone PR view. The list title
/// counts PRs: `chaoshub · 1 PR to review`, `chaoshub · 3 PRs to review`.
/// The count is the ready set of the Review tab, the same set that the
/// notice of new pull requests watches. The view filters do not change it. Count once
/// per frame generation, not on each terminal write. While the list
/// syncs or prefetches, a spinner follows the count. The words are in
/// the status bar. When the terminal shows its own spinner for the
/// program status (`title_spinner` is false), the title has only the
/// count.
pub(crate) fn term_title(state: &AppState, status: &TermStatus) -> String {
    let data = &state.data;
    let mut title = if let Some(key) = &state.ui.pinned {
        match data.prs.get(key) {
            Some(pr) => pr_overview(pr),
            None => format!("#{}", key.number),
        }
    } else if !data.synced.contains_key(&Scope::Team) {
        "chaoshub · ? PRs to review".to_owned()
    } else {
        let count = ready_count(data, "review");
        format!("chaoshub · {} to review", plural::count(count, "PR", "PRs"))
    };
    let activity = match status.state {
        TermState::Idle | TermState::Clear => None,
        TermState::Blocked { .. } => Some("input".to_owned()),
        TermState::Error => Some("error".to_owned()),
        TermState::Done => status.msg.clone(),
        TermState::Working if data.merge.is_some() => status.msg.clone(),
        TermState::Working if state.ui.pinned.is_none() => data
            .title_spinner
            .then(|| spinner_at(data.now, TITLE_SPINNER_FRAME).to_owned()),
        TermState::Working => state
            .ui
            .pinned
            .as_ref()
            .filter(|key| !data.prs.contains_key(key))
            .map(|_| "loading".to_owned()),
    };
    if let Some(activity) = activity {
        title.push_str(" · ");
        title.push_str(&activity);
    }
    title
}

/// A short overview of a pull request for the title of the standalone PR
/// view: `#7 <title> · <verdict> · <CI> · <reviews>`. The verdict is the
/// badge of the description pane. A merged or closed pull request shows
/// only the number, the title and the verdict. The CI and the reviews use the words of the merge
/// confirm popup. They are not in the title when the pull request has no
/// checks or needs no review.
fn pr_overview(pr: &PullRequest) -> String {
    let head = &pr.head.value;
    let (badge, _) = description::badge(head, pr.ci_state.value);
    let mut parts = vec![
        format!("#{} {}", pr.key.number, short(head.title.trim())),
        badge.to_lowercase(),
    ];
    if head.state == PrState::Open {
        parts.extend(ci_text(pr.ci_state.value).map(str::to_owned));
        parts.extend(review_text(head).map(str::to_owned));
    }
    parts.join(" · ")
}

/// The CI state: `CI passed`, `CI failed` or `CI running`. `None` without
/// checks.
fn ci_text(state: CiState) -> Option<&'static str> {
    match state {
        CiState::Success => Some("CI passed"),
        CiState::Failure => Some("CI failed"),
        CiState::Pending => Some("CI running"),
        CiState::None => None,
    }
}

/// The review state: `approved`, `changes requested` or `review
/// required`. Without a review decision, the reviews count: `approved`
/// with approvals, `changes requested` with change requests, else `None`.
fn review_text(head: &PrHead) -> Option<&'static str> {
    match head.review_decision {
        Some(ReviewDecision::Approved) => Some("approved"),
        Some(ReviewDecision::ChangesRequested) => Some("changes requested"),
        Some(ReviewDecision::ReviewRequired) => Some("review required"),
        None if head.changes_requested > 0 => Some("changes requested"),
        None if head.approvals > 0 => Some("approved"),
        None => None,
    }
}

/// Cuts a PR title to `PR_TITLE_MAX` chars. A cut title ends with `…`.
fn short(title: &str) -> String {
    if title.chars().count() <= PR_TITLE_MAX {
        return title.to_owned();
    }
    let mut out: String = title.chars().take(PR_TITLE_MAX - 1).collect();
    out.truncate(out.trim_end().len());
    out.push('…');
    out
}

/// The terminal status without the overlays.
fn base_status(state: &AppState) -> TermStatus {
    let data = &state.data;
    if let Some(merge) = &data.merge {
        return merge_status(merge, data.now);
    }
    if let Some(failure) = &data.sync.failure {
        let progress = if failure.rate_limited_until.is_some() {
            Progress::Paused
        } else {
            error_progress(failure.at, data.now)
        };
        return TermStatus {
            state: TermState::Error,
            msg: Some(failure.reason.clone()),
            progress,
        };
    }
    if syncing(state) {
        return TermStatus {
            state: TermState::Working,
            msg: Some(format!(
                "syncing {}",
                plural::count(data.session.repos.len(), "repo", "repos")
            )),
            progress: Progress::Indeterminate,
        };
    }
    if let Some(batch) = data.prefetch.filter(|b| b.total > 1 && b.done < b.total) {
        let percent = (batch.done * 100 / batch.total).min(100);
        return TermStatus {
            state: TermState::Working,
            msg: Some("prefetching".to_owned()),
            progress: Progress::Percent(u8::try_from(percent).unwrap_or(100)),
        };
    }
    TermStatus {
        state: TermState::Idle,
        msg: None,
        progress: Progress::Off,
    }
}

/// The terminal status of an async merge.
fn merge_status(merge: &MergeState, now: DateTime<Utc>) -> TermStatus {
    let number = merge.key.number;
    let (state, msg, progress) = match &merge.progress {
        MergeProgress::Pending => (
            TermState::Working,
            format!("merging #{number}"),
            Progress::Indeterminate,
        ),
        MergeProgress::Enqueued => (
            TermState::Working,
            format!("#{number} in merge queue"),
            Progress::Indeterminate,
        ),
        MergeProgress::Merged { .. } => {
            let progress = if now < merge.at + DONE_PROGRESS {
                Progress::Percent(100)
            } else {
                Progress::Off
            };
            (TermState::Done, format!("merged #{number}"), progress)
        }
        MergeProgress::Failed(reason) => (
            TermState::Error,
            format!("merge of #{number} failed: {reason}"),
            error_progress(merge.at, now),
        ),
    };
    TermStatus {
        state,
        msg: Some(msg),
        progress,
    }
}

/// The progress bar after a failure at `at`: error for 5 s, then off.
fn error_progress(at: DateTime<Utc>, now: DateTime<Utc>) -> Progress {
    if now < at + ERROR_PROGRESS {
        Progress::Error
    } else {
        Progress::Off
    }
}

/// The title of the open confirm or prompt overlay nearest the top.
fn question(state: &AppState) -> Option<String> {
    state
        .ui
        .overlays
        .iter()
        .rev()
        .filter(|overlay| !overlay.closing)
        .find_map(|overlay| match &overlay.state {
            OverlayState::Confirm { spec, .. } => Some(spec.title.clone()),
            OverlayState::Prompt { spec, .. } => Some(spec.title.clone()),
            OverlayState::Review(review) => Some(review.title()),
            OverlayState::ReviewText { title, .. } => Some(title.clone()),
            _ => None,
        })
}

/// True while a requested sync runs, or before the first sync of a scope
/// once the login is known.
fn syncing(state: &AppState) -> bool {
    let data = &state.data;
    let first = data.session.login.is_some()
        && [Scope::Mine, Scope::Team]
            .iter()
            .any(|scope| !data.synced.contains_key(scope));
    first || !data.sync.pending.is_empty()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::app::frame;
    use crate::app::models::RowItem;
    use crate::app::state::{ActionDialog, Failure, Overlay, PaneHandleState, PrefetchBatch};
    use crate::app::tabs::TABS;
    use crate::app::testdata;
    use crate::bus::{ConfirmSpec, OverlayKind, PromptSpec, Selection, Size, Target, TargetKey};
    use crate::domain::local::LocalMatch;
    use crate::domain::pr::{MergeStateStatus, Mergeable};
    use crate::domain::repo::Repo;
    use crate::domain::test_time;

    fn at(secs: i64) -> DateTime<Utc> {
        test_time() + TimeDelta::seconds(secs)
    }

    fn status(state: TermState, msg: Option<&str>, progress: Progress) -> TermStatus {
        TermStatus {
            state,
            msg: msg.map(str::to_owned),
            progress,
        }
    }

    fn failure(rate_limited: bool) -> Failure {
        Failure {
            source: "sync",
            reason: if rate_limited {
                "rate limited until 14:05".into()
            } else {
                "sync failed: offline".into()
            },
            rate_limited_until: rate_limited.then(|| at(600)),
            at: at(0),
        }
    }

    type Setup = fn(&mut AppState);

    /// The rows of the status table: a name, a setup and the status.
    fn cases() -> [(&'static str, Setup, TermStatus); 8] {
        [
            (
                "fresh state is idle",
                |_| {},
                status(TermState::Idle, None, Progress::Off),
            ),
            (
                "first sync after the login",
                |s| s.data.session.login = Some("me".into()),
                status(
                    TermState::Working,
                    Some("syncing 2 repos"),
                    Progress::Indeterminate,
                ),
            ),
            (
                "both scopes synced",
                |s| {
                    s.data.session.login = Some("me".into());
                    s.data.synced.insert(Scope::Mine, at(0));
                    s.data.synced.insert(Scope::Team, at(0));
                },
                status(TermState::Idle, None, Progress::Off),
            ),
            (
                "requested sync",
                |s| {
                    s.data.sync.pending.insert(Scope::Team);
                },
                status(
                    TermState::Working,
                    Some("syncing 2 repos"),
                    Progress::Indeterminate,
                ),
            ),
            (
                "sync failed, first 5 s",
                |s| {
                    s.data.sync.failure = Some(failure(false));
                    s.data.now = at(4);
                },
                status(
                    TermState::Error,
                    Some("sync failed: offline"),
                    Progress::Error,
                ),
            ),
            (
                "sync failed, after 5 s",
                |s| {
                    s.data.sync.failure = Some(failure(false));
                    s.data.now = at(5);
                },
                status(
                    TermState::Error,
                    Some("sync failed: offline"),
                    Progress::Off,
                ),
            ),
            (
                "failure wins over a running sync",
                |s| {
                    s.data.sync.pending.insert(Scope::Mine);
                    s.data.sync.failure = Some(failure(false));
                    s.data.now = at(1);
                },
                status(
                    TermState::Error,
                    Some("sync failed: offline"),
                    Progress::Error,
                ),
            ),
            (
                "rate limited",
                |s| {
                    s.data.sync.failure = Some(failure(true));
                    s.data.now = at(60);
                },
                status(
                    TermState::Error,
                    Some("rate limited until 14:05"),
                    Progress::Paused,
                ),
            ),
        ]
    }

    fn merge(progress: MergeProgress) -> MergeState {
        MergeState {
            progress,
            ..MergeState::new(crate::app::testdata::pr_key("a", 7), at(0))
        }
    }

    fn dialog() -> ActionDialog {
        ActionDialog {
            id: "merge",
            selection: Box::new(Selection {
                target: Target::Pr {
                    pr: crate::app::testdata::pr("a", 7, "Add the thing"),
                    stack: Vec::new(),
                    local: LocalMatch::None,
                    repo: Repo {
                        key: crate::app::testdata::repo_key("a"),
                        path: None,
                        settings: None,
                    },
                },
                me: "me".into(),
                action_config: None,
            }),
        }
    }

    fn overlay(state: OverlayState, closing: bool) -> Overlay {
        let kind = match state {
            OverlayState::Prompt { .. } => OverlayKind::Prompt,
            _ => OverlayKind::Confirm,
        };
        Overlay {
            kind,
            closing,
            state,
        }
    }

    fn confirm(title: &str) -> OverlayState {
        OverlayState::Confirm {
            action: dialog(),
            spec: ConfirmSpec {
                title: title.into(),
                lines: Vec::new(),
                danger: true,
            },
        }
    }

    fn blocked(msg: &str, progress: Progress) -> TermStatus {
        status(
            TermState::Blocked {
                kind: BlockedKind::Question,
            },
            Some(msg),
            progress,
        )
    }

    /// The rows of the status table for merges.
    fn merge_cases() -> Vec<(&'static str, Setup, TermStatus)> {
        vec![
            (
                "merge pending",
                |s| s.data.merge = Some(merge(MergeProgress::Pending)),
                status(
                    TermState::Working,
                    Some("merging #7"),
                    Progress::Indeterminate,
                ),
            ),
            (
                "merge enqueued",
                |s| s.data.merge = Some(merge(MergeProgress::Enqueued)),
                status(
                    TermState::Working,
                    Some("#7 in merge queue"),
                    Progress::Indeterminate,
                ),
            ),
            (
                "merged, first 2 s",
                |s| {
                    s.data.merge = Some(merge(MergeProgress::Merged { sha: "abc".into() }));
                    s.data.now = at(1);
                },
                status(TermState::Done, Some("merged #7"), Progress::Percent(100)),
            ),
            (
                "merged, after 2 s",
                |s| {
                    s.data.merge = Some(merge(MergeProgress::Merged { sha: "abc".into() }));
                    s.data.now = at(2);
                },
                status(TermState::Done, Some("merged #7"), Progress::Off),
            ),
            (
                "merge failed, first 5 s",
                |s| {
                    s.data.merge = Some(merge(MergeProgress::Failed("checks failing".into())));
                    s.data.now = at(4);
                },
                status(
                    TermState::Error,
                    Some("merge of #7 failed: checks failing"),
                    Progress::Error,
                ),
            ),
            (
                "merge failed, after 5 s",
                |s| {
                    s.data.merge = Some(merge(MergeProgress::Failed("checks failing".into())));
                    s.data.now = at(5);
                },
                status(
                    TermState::Error,
                    Some("merge of #7 failed: checks failing"),
                    Progress::Off,
                ),
            ),
        ]
    }

    /// The rows of the status table for overlays, prefetch and panes.
    fn other_cases() -> Vec<(&'static str, Setup, TermStatus)> {
        vec![
            (
                "confirm overlay open",
                |s| s.ui.overlays.push(overlay(confirm("merge #7?"), false)),
                blocked("merge #7?", Progress::Off),
            ),
            (
                "prompt overlay open keeps the progress",
                |s| {
                    s.data.sync.pending.insert(Scope::Mine);
                    let prompt = OverlayState::Prompt {
                        action: dialog(),
                        spec: PromptSpec {
                            title: "request changes on #7".into(),
                            placeholder: String::new(),
                        },
                        text: String::new(),
                    };
                    s.ui.overlays.push(overlay(prompt, false));
                },
                blocked("request changes on #7", Progress::Indeterminate),
            ),
            (
                "closing confirm overlay",
                |s| s.ui.overlays.push(overlay(confirm("merge #7?"), true)),
                status(TermState::Idle, None, Progress::Off),
            ),
            (
                "sync of one repo uses the singular",
                |s| {
                    s.data.session.repos.truncate(1);
                    s.data.sync.pending.insert(Scope::Team);
                },
                status(
                    TermState::Working,
                    Some("syncing 1 repo"),
                    Progress::Indeterminate,
                ),
            ),
            (
                "prefetch batch",
                |s| s.data.prefetch = Some(PrefetchBatch { done: 1, total: 4 }),
                status(
                    TermState::Working,
                    Some("prefetching"),
                    Progress::Percent(25),
                ),
            ),
            (
                "prefetch of one ref is quiet",
                |s| s.data.prefetch = Some(PrefetchBatch { done: 0, total: 1 }),
                status(TermState::Idle, None, Progress::Off),
            ),
            (
                "finished prefetch batch is quiet",
                |s| s.data.prefetch = Some(PrefetchBatch { done: 4, total: 4 }),
                status(TermState::Idle, None, Progress::Off),
            ),
            (
                "review pane open is idle",
                |s| {
                    let key = TargetKey::Pr(crate::app::testdata::pr_key("a", 7));
                    s.data
                        .panes
                        .insert(("review", key), PaneHandleState::Running);
                },
                status(TermState::Idle, None, Progress::Off),
            ),
        ]
    }

    #[test]
    fn term_status_table() {
        for (name, setup, expected) in cases()
            .into_iter()
            .chain(merge_cases())
            .chain(other_cases())
        {
            let mut state = AppState::new(Size {
                width: 100,
                height: 30,
            });
            let repo = crate::app::testdata::repo_key;
            state.data.session.repos = ["a", "b"]
                .iter()
                .map(|name| crate::domain::repo::Repo {
                    key: repo(name),
                    path: None,
                    settings: None,
                })
                .collect();
            setup(&mut state);
            assert_eq!(term_status(&state), expected, "{name}");
        }
    }

    fn state() -> AppState {
        let mut state = AppState::new(Size {
            width: 100,
            height: 30,
        });
        state.data.now = test_time();
        for scope in [Scope::Mine, Scope::Team] {
            state.data.synced.insert(scope, test_time());
        }
        state
    }

    fn title(state: &AppState) -> String {
        term_title(state, &term_status(state))
    }

    #[test]
    fn the_title_count_matches_the_normal_review_tab() {
        let mut state = state();
        state.data.filters.max_age = Some(TimeDelta::weeks(2));
        let mut prs: Vec<_> = (1..=13)
            .map(|number| {
                let mut pr = testdata::pr("chaoshub", number, "Review me");
                pr.head.value.review_requested = true;
                pr.ci_state.value = CiState::Success;
                pr
            })
            .collect();
        prs[1].head.value.review_requested = false;
        prs[3].ci_state.value = CiState::Pending;
        prs[4].ci_state.value = CiState::Failure;
        prs[5].ci_state.value = CiState::None;
        prs[6].head.value.is_draft = true;
        prs[7].head.value.state = PrState::Closed;
        prs[8].head.value.state = PrState::Merged;
        prs[9].head.value.created_at = Some(test_time() - TimeDelta::weeks(3));
        prs[10].head.value.created_at = None;
        prs[11].head.value.created_at = Some(test_time() - TimeDelta::weeks(2));
        prs[12].head.value.author = "another-reviewer".into();
        let snapshot = crate::store::Snapshot {
            prs: prs
                .into_iter()
                .map(|pr| {
                    let scope = if pr.key.number == 3 {
                        Scope::Mine
                    } else {
                        Scope::Team
                    };
                    (scope, pr)
                })
                .collect(),
            ..crate::store::Snapshot::default()
        };
        state.load(snapshot, Vec::new());
        for scope in [Scope::Mine, Scope::Team] {
            state.data.synced.insert(scope, test_time());
        }
        state.ui.active = TABS.iter().position(|tab| tab.id() == "review").unwrap();
        let frame = frame::frame_model(&mut state);
        let numbers: Vec<_> = frame
            .list
            .rows
            .iter()
            .filter_map(|row| {
                if let RowItem::Pr(pr) = &row.item {
                    Some(pr.key.number)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(numbers, [1, 12, 13]);
        assert_eq!(frame.title, "chaoshub · 3 PRs to review");
        for active in 0..TABS.len() {
            state.ui.active = active;
            for tab in &mut state.ui.tabs {
                tab.author = Some("nobody".into());
                tab.filter = "no match".into();
                tab.show_hidden = true;
            }
            assert_eq!(title(&state), "chaoshub · 3 PRs to review");
        }
    }

    #[test]
    fn the_title_count_uses_the_singular_for_one_pr() {
        let mut state = state();
        let mut pr = testdata::pr("chaoshub", 1, "Review me");
        pr.head.value.review_requested = true;
        pr.ci_state.value = CiState::Success;
        let snapshot = crate::store::Snapshot {
            prs: vec![(Scope::Team, pr)],
            ..crate::store::Snapshot::default()
        };
        state.load(snapshot, Vec::new());
        for scope in [Scope::Mine, Scope::Team] {
            state.data.synced.insert(scope, test_time());
        }
        assert_eq!(title(&state), "chaoshub · 1 PR to review");
    }

    #[test]
    fn an_unknown_count_is_not_reported_as_zero() {
        let mut state = state();
        state.data.synced.remove(&Scope::Team);
        assert_eq!(title(&state), "chaoshub · ? PRs to review");
        state.data.session.login = Some("me".into());
        assert_eq!(
            title(&state),
            format!("chaoshub · ? PRs to review · {}", title_spinner(&state))
        );
        state.data.synced.insert(Scope::Team, test_time());
        assert_eq!(title(&state), "chaoshub · 0 PRs to review");
    }

    #[test]
    fn the_title_includes_one_short_activity() {
        let mut state = state();
        state.data.prefetch = Some(PrefetchBatch { done: 1, total: 2 });
        assert_eq!(
            title(&state),
            format!("chaoshub · 0 PRs to review · {}", title_spinner(&state))
        );
        state.data.merge = Some(MergeState::new(
            testdata::pr_key("chaoshub", 42),
            test_time(),
        ));
        assert_eq!(title(&state), "chaoshub · 0 PRs to review · merging #42");
        let blocked = TermStatus {
            state: TermState::Blocked {
                kind: BlockedKind::Question,
            },
            msg: Some("A long question that does not belong in the title".into()),
            progress: Progress::Indeterminate,
        };
        assert_eq!(
            term_title(&state, &blocked),
            "chaoshub · 0 PRs to review · input"
        );
        let error = TermStatus {
            state: TermState::Error,
            msg: Some("A long error that does not belong in the title".into()),
            progress: Progress::Error,
        };
        assert_eq!(
            term_title(&state, &error),
            "chaoshub · 0 PRs to review · error"
        );
    }

    #[test]
    fn a_terminal_with_its_own_spinner_gets_no_title_spinner() {
        let mut state = state();
        state.data.title_spinner = false;
        assert_eq!(title(&state), "chaoshub · 0 PRs to review", "idle");
        state.data.sync.pending.insert(Scope::Mine);
        assert_eq!(term_status(&state).state, TermState::Working);
        assert_eq!(title(&state), "chaoshub · 0 PRs to review", "syncing");
        state.data.sync.pending.clear();
        state.data.prefetch = Some(PrefetchBatch { done: 1, total: 2 });
        assert_eq!(term_status(&state).state, TermState::Working);
        assert_eq!(title(&state), "chaoshub · 0 PRs to review", "prefetching");
        state.data.title_spinner = true;
        assert_eq!(
            title(&state),
            format!("chaoshub · 0 PRs to review · {}", title_spinner(&state)),
            "the other terminals"
        );
        state.data.prefetch = None;
        assert_eq!(title(&state), "chaoshub · 0 PRs to review", "idle again");
    }

    #[test]
    fn a_terminal_with_its_own_spinner_keeps_the_word_activities() {
        let mut state = state();
        state.data.title_spinner = false;
        state.data.merge = Some(MergeState::new(
            testdata::pr_key("chaoshub", 42),
            test_time(),
        ));
        assert_eq!(title(&state), "chaoshub · 0 PRs to review · merging #42");
        state.data.merge = None;
        let blocked = TermStatus {
            state: TermState::Blocked {
                kind: BlockedKind::Question,
            },
            msg: Some("A question".into()),
            progress: Progress::Indeterminate,
        };
        assert_eq!(
            term_title(&state, &blocked),
            "chaoshub · 0 PRs to review · input"
        );
        let error = TermStatus {
            state: TermState::Error,
            msg: Some("An error".into()),
            progress: Progress::Error,
        };
        assert_eq!(
            term_title(&state, &error),
            "chaoshub · 0 PRs to review · error"
        );
        let done = TermStatus {
            state: TermState::Done,
            msg: Some("merged #42".into()),
            progress: Progress::Percent(100),
        };
        assert_eq!(
            term_title(&state, &done),
            "chaoshub · 0 PRs to review · merged #42"
        );
    }

    /// The frame of the title spinner at the time of `state`.
    fn title_spinner(state: &AppState) -> &'static str {
        spinner_at(state.data.now, TITLE_SPINNER_FRAME)
    }

    #[test]
    fn the_title_spinner_moves_at_half_the_rate_of_the_status_bar() {
        // `test_time` is a whole second, so a title frame starts at it.
        let mut state = state();
        state.data.sync.pending.insert(Scope::Mine);
        let first = title(&state);
        assert_eq!(first, "chaoshub · 0 PRs to review · ⠋");
        state.data.now += SPINNER_FRAME;
        assert_eq!(title(&state), first, "one status bar frame later");
        state.data.now += SPINNER_FRAME;
        assert_eq!(title(&state), "chaoshub · 0 PRs to review · ⠙");
        let titles: HashSet<String> = (0..20)
            .map(|_| {
                state.data.now += SPINNER_FRAME;
                title(&state)
            })
            .collect();
        assert_eq!(titles.len(), 10, "the frames of the status bar spinner");
    }

    /// A state with the standalone PR view of `pr`.
    fn pinned(pr: crate::domain::pr::PullRequest) -> AppState {
        let mut state = state();
        state.ui.pinned = Some(pr.key.clone());
        state.data.prs.insert(pr.key.clone(), pr);
        state
    }

    /// An open pull request that is ready to merge: green CI, approved,
    /// no conflicts.
    fn ready_pr() -> crate::domain::pr::PullRequest {
        let mut pr = testdata::pr("chaoshub", 7, "Fix the merge");
        pr.ci_state.value = CiState::Success;
        let head = &mut pr.head.value;
        head.review_decision = Some(ReviewDecision::Approved);
        head.approvals = 2;
        head.mergeable = Mergeable::Mergeable;
        head.merge_state = MergeStateStatus::Clean;
        pr
    }

    #[test]
    fn a_standalone_pr_title_gives_an_overview_without_a_list_count() {
        let mut state = pinned(ready_pr());
        assert_eq!(
            title(&state),
            "#7 Fix the merge · ready · CI passed · approved"
        );
        state.data.synced.clear();
        state.data.session.login = Some("me".into());
        assert_eq!(
            title(&state),
            "#7 Fix the merge · ready · CI passed · approved"
        );
        state.data.prs.clear();
        assert_eq!(title(&state), "#7 · loading");
    }

    #[test]
    fn the_pr_title_shows_each_verdict() {
        type Change = fn(&mut crate::domain::pr::PullRequest);
        let cases: [(&str, Change, &str); 12] = [
            (
                "ready",
                |_| {},
                "#7 Fix the merge · ready · CI passed · approved",
            ),
            (
                "failed CI",
                |pr| pr.ci_state.value = CiState::Failure,
                "#7 Fix the merge · blocked · CI failed · approved",
            ),
            (
                "running CI",
                |pr| pr.ci_state.value = CiState::Pending,
                "#7 Fix the merge · waiting · CI running · approved",
            ),
            (
                "requested changes",
                |pr| {
                    pr.head.value.review_decision = Some(ReviewDecision::ChangesRequested);
                },
                "#7 Fix the merge · blocked · CI passed · changes requested",
            ),
            (
                "required review",
                |pr| {
                    pr.head.value.review_decision = Some(ReviewDecision::ReviewRequired);
                    pr.head.value.approvals = 0;
                },
                "#7 Fix the merge · blocked · CI passed · review required",
            ),
            (
                "conflicts",
                |pr| pr.head.value.mergeable = Mergeable::Conflicting,
                "#7 Fix the merge · conflicts · CI passed · approved",
            ),
            (
                "unknown merge state",
                |pr| pr.head.value.mergeable = Mergeable::Unknown,
                "#7 Fix the merge · checking · CI passed · approved",
            ),
            (
                "draft",
                |pr| pr.head.value.is_draft = true,
                "#7 Fix the merge · draft · CI passed · approved",
            ),
            (
                "merge queue",
                |pr| pr.head.value.in_merge_queue = true,
                "#7 Fix the merge · in queue · CI passed · approved",
            ),
            (
                "no checks and no review decision",
                |pr| {
                    pr.ci_state.value = CiState::None;
                    pr.head.value.review_decision = None;
                    pr.head.value.approvals = 0;
                },
                "#7 Fix the merge · ready",
            ),
            (
                "merged",
                |pr| pr.head.value.state = PrState::Merged,
                "#7 Fix the merge · merged",
            ),
            (
                "closed",
                |pr| pr.head.value.state = PrState::Closed,
                "#7 Fix the merge · closed",
            ),
        ];
        for (name, change, expected) in cases {
            let mut pr = ready_pr();
            change(&mut pr);
            assert_eq!(title(&pinned(pr)), expected, "{name}");
        }
    }

    #[test]
    fn the_pr_title_counts_reviews_without_a_review_decision() {
        let mut pr = ready_pr();
        pr.head.value.review_decision = None;
        assert_eq!(
            title(&pinned(pr.clone())),
            "#7 Fix the merge · ready · CI passed · approved"
        );
        pr.head.value.changes_requested = 1;
        assert_eq!(
            title(&pinned(pr)),
            "#7 Fix the merge · ready · CI passed · changes requested"
        );
    }

    #[test]
    fn a_long_pr_title_is_cut() {
        let mut pr = ready_pr();
        pr.head.value.title = "  Add the terminal title overview to the PR view  ".into();
        assert_eq!(
            title(&pinned(pr)),
            "#7 Add the terminal title overvi… · ready · CI passed · approved"
        );
        let mut pr = ready_pr();
        pr.head.value.title = "1234567890123456789012345678 cut after the space".into();
        assert_eq!(
            title(&pinned(pr)),
            "#7 1234567890123456789012345678… · ready · CI passed · approved"
        );
    }

    #[test]
    fn the_pr_title_keeps_the_activity() {
        let mut state = pinned(ready_pr());
        state.data.merge = Some(MergeState::new(
            testdata::pr_key("chaoshub", 7),
            test_time(),
        ));
        assert_eq!(
            title(&state),
            "#7 Fix the merge · ready · CI passed · approved · merging #7"
        );
        let error = TermStatus {
            state: TermState::Error,
            msg: Some("merge of #7 failed: checks failing".into()),
            progress: Progress::Error,
        };
        assert_eq!(
            term_title(&state, &error),
            "#7 Fix the merge · ready · CI passed · approved · error"
        );
    }
}
