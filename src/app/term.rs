//! Derives the terminal status from the app state.
//!
//! `term_status` is pure. It is part of the `FrameModel`, so it is rebuilt
//! only with the generation. It follows the status table of section 1.14
//! of the plan:
//!
//! | Situation | OSC 7501 | OSC 9;4 |
//! |---|---|---|
//! | Idle | `idle` | off |
//! | First sync or requested sync running | `working`, `syncing <n> repos` | indeterminate |
//! | Sync failed, worker died | `error`, the reason | error for 5 s, then off |
//! | Rate limited | `error`, `rate limited until HH:MM` | paused |
//!
//! A failure stays until the next key press or the next successful sync.

use chrono::TimeDelta;

use crate::app::state::AppState;
use crate::bus::{Progress, Scope, TermState, TermStatus};

/// How long the progress bar shows an error after a failure.
pub(crate) const ERROR_PROGRESS: TimeDelta = TimeDelta::seconds(5);

/// The terminal status for the app state.
pub(crate) fn term_status(state: &AppState) -> TermStatus {
    let data = &state.data;
    if let Some(failure) = &data.sync.failure {
        let progress = if failure.rate_limited_until.is_some() {
            Progress::Paused
        } else if data.now < failure.at + ERROR_PROGRESS {
            Progress::Error
        } else {
            Progress::Off
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
    TermStatus {
        state: TermState::Idle,
        msg: None,
        progress: Progress::Off,
    }
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
    use chrono::{DateTime, Utc};

    use super::*;
    use crate::app::state::Failure;
    use crate::bus::Size;
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

    #[test]
    fn term_status_table() {
        for (name, setup, expected) in cases() {
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
