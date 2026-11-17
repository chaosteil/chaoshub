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
//! | First sync or requested sync running | `working`, `syncing <n> repos` | indeterminate |
//! | Prefetch batch of more than one ref | `working`, `prefetching` | the percent done |
//! | Sync failed, worker died | `error`, the reason | error for 5 s, then off |
//! | Rate limited | `error`, `rate limited until HH:MM` | paused |
//! | Review pane open | `idle`: the pane is the user's | unchanged |
//!
//! A failure stays until the next key press or the next successful sync.
//! A final merge state stays until the next key press after the end of
//! the run, when the pull request shows the end.

use chrono::{DateTime, TimeDelta, Utc};

use crate::app::notify::ready_count;
use crate::app::state::{AppState, MergeState, OverlayState};
use crate::bus::{BlockedKind, MergeProgress, Progress, Scope, TermState, TermStatus};

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

/// The title of the list or the standalone PR view. The review count is
/// the ready set of the Review tab, the same set that the notice of new
/// pull requests watches. The view filters do not change it. Count once
/// per frame generation, not on each terminal write.
pub(crate) fn term_title(state: &AppState, status: &TermStatus) -> String {
    let data = &state.data;
    let mut title = if let Some(key) = &state.ui.pinned {
        format!(
            "chaoshub · {}/{}#{}",
            key.repo.owner, key.repo.name, key.number
        )
    } else if !data.synced.contains_key(&Scope::Team) {
        "chaoshub · ? review".to_owned()
    } else {
        format!("chaoshub · {} review", ready_count(data, "review"))
    };
    let activity = match status.state {
        TermState::Idle | TermState::Clear => None,
        TermState::Blocked { .. } => Some("input".to_owned()),
        TermState::Error => Some("error".to_owned()),
        TermState::Done => status.msg.clone(),
        TermState::Working if data.merge.is_some() => status.msg.clone(),
        TermState::Working if state.ui.pinned.is_none() => Some(
            if syncing(state) {
                "syncing"
            } else {
                "prefetching"
            }
            .to_owned(),
        ),
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
            msg: Some(format!("syncing {} repos", data.session.repos.len())),
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
    use super::*;
    use crate::app::state::{ActionDialog, Failure, Overlay, PaneHandleState, PrefetchBatch};
    use crate::bus::{ConfirmSpec, OverlayKind, PromptSpec, Selection, Size, Target, TargetKey};
    use crate::domain::local::LocalMatch;
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
}

#[cfg(test)]
mod title_tests {
    use super::*;
    use crate::app::frame;
    use crate::app::models::RowItem;
    use crate::app::state::PrefetchBatch;
    use crate::app::tabs::TABS;
    use crate::app::testdata;
    use crate::bus::Size;
    use crate::domain::pr::{CiState, PrState};
    use crate::domain::test_time;

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
        assert_eq!(frame.title, "chaoshub · 3 review");
        for active in 0..TABS.len() {
            state.ui.active = active;
            for tab in &mut state.ui.tabs {
                tab.author = Some("nobody".into());
                tab.filter = "no match".into();
                tab.show_hidden = true;
            }
            assert_eq!(title(&state), "chaoshub · 3 review");
        }
    }

    #[test]
    fn an_unknown_count_is_not_reported_as_zero() {
        let mut state = state();
        state.data.synced.remove(&Scope::Team);
        assert_eq!(title(&state), "chaoshub · ? review");
        state.data.session.login = Some("me".into());
        assert_eq!(title(&state), "chaoshub · ? review · syncing");
        state.data.synced.insert(Scope::Team, test_time());
        assert_eq!(title(&state), "chaoshub · 0 review");
    }

    #[test]
    fn the_title_includes_one_short_activity() {
        let mut state = state();
        state.data.prefetch = Some(PrefetchBatch { done: 1, total: 2 });
        assert_eq!(title(&state), "chaoshub · 0 review · prefetching");
        state.data.merge = Some(MergeState::new(
            testdata::pr_key("chaoshub", 42),
            test_time(),
        ));
        assert_eq!(title(&state), "chaoshub · 0 review · merging #42");
        let blocked = TermStatus {
            state: TermState::Blocked {
                kind: BlockedKind::Question,
            },
            msg: Some("A long question that does not belong in the title".into()),
            progress: Progress::Indeterminate,
        };
        assert_eq!(term_title(&state, &blocked), "chaoshub · 0 review · input");
        let error = TermStatus {
            state: TermState::Error,
            msg: Some("A long error that does not belong in the title".into()),
            progress: Progress::Error,
        };
        assert_eq!(term_title(&state, &error), "chaoshub · 0 review · error");
    }

    #[test]
    fn a_standalone_pr_title_identifies_the_pr_without_a_list_count() {
        let mut state = state();
        let pr = testdata::pr("chaoshub", 7, "A PR title stays in the pane");
        state.ui.pinned = Some(pr.key.clone());
        state.data.prs.insert(pr.key.clone(), pr);
        assert_eq!(title(&state), "chaoshub · chaosteil/chaoshub#7");
        state.data.synced.clear();
        state.data.session.login = Some("me".into());
        assert_eq!(title(&state), "chaoshub · chaosteil/chaoshub#7");
        state.data.prs.clear();
        assert_eq!(title(&state), "chaoshub · chaosteil/chaoshub#7 · loading");
    }
}
