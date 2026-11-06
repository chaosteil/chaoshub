//! Derives the terminal status from the app state.
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
//! A final merge state stays until the next key press.

use chrono::{DateTime, TimeDelta, Utc};

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
            OverlayState::Choose { spec, .. } => Some(spec.title.clone()),
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
            key: crate::app::testdata::pr_key("a", 7),
            progress,
            at: at(0),
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
