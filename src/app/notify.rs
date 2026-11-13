//! The notification of new pull requests that are ready for review.
//!
//! A pull request is new when it enters the ready set of a watched tab
//! and chaoshub did not announce it before. The ready set of a tab holds
//! its open pull requests that are not drafts, have green CI and are in
//! the age limit. The author filter, the filter text and the `t` switch
//! do not change the ready set.
//!
//! The cache load and the first sync of each scope only set the ready
//! sets, the same as the row glow. The announced pull requests stay in
//! the `ui_state` table, so a restart does not announce them again.
//!
//! A new pull request starts the debounce wait, and each further one
//! starts it again. The wait ends after the debounce time without a new
//! pull request, or 5 times the debounce time after the first one. Then
//! one notice announces all of them.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use chrono::{DateTime, TimeDelta, Utc};

use crate::app::state::AppData;
use crate::app::tabs::pr_ref;
use crate::bus::{Cmd, Persist, PrRef, Scope};
use crate::config::schema::NOTIFY_TABS;
use crate::domain::keys::PrKey;
use crate::domain::pr::{CiState, PrState, PullRequest};
use crate::domain::team::is_ready;

/// The `ui_state` key of the announced pull requests: a JSON list of
/// pull request keys.
pub(crate) const ANNOUNCED_KEY: &str = "notify.announced";
/// The title of every notice.
const TITLE: &str = "chaoshub";
/// The longest wait after the first new pull request, in debounce times.
const CAP: i32 = 5;

/// The `[notify]` settings of a session where the notification is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NotifySettings {
    /// The ids of the watched tabs.
    pub tabs: Vec<String>,
    /// The wait after a new pull request.
    pub debounce: TimeDelta,
}

/// The notice that the terminal shows. `seq` counts the notices, so two
/// notices with the same text are two writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    pub seq: u64,
    pub title: String,
    pub msg: String,
}

/// The state of the notification.
#[derive(Debug, Default)]
pub(crate) struct NotifyState {
    /// `None` while the notification is off.
    pub settings: Option<NotifySettings>,
    /// The ready set of each watched tab after the last check.
    ready: BTreeMap<String, BTreeSet<PrKey>>,
    /// The pull requests that chaoshub announced, or that wait for the
    /// end of the debounce wait.
    pub announced: BTreeSet<PrKey>,
    /// The new pull requests that wait, with their tab, in the order of
    /// their arrival.
    pending: Vec<(String, PrKey)>,
    /// The arrival of the first and of the last pending pull request.
    first_at: Option<DateTime<Utc>>,
    last_at: Option<DateTime<Utc>>,
    /// The notice that the terminal shows, until the next key press.
    pub notice: Option<Notice>,
    /// The number of notices so far.
    seq: u64,
}

impl NotifyState {
    /// Reads the announced pull requests from the `ui_state` value. A
    /// value that does not parse gives none.
    pub(crate) fn load_announced(&mut self, value: &str) {
        match serde_json::from_str::<Vec<PrKey>>(value) {
            Ok(keys) => self.announced = keys.into_iter().collect(),
            Err(err) => tracing::warn!(%err, "cannot read the announced pull requests"),
        }
    }

    /// The end of the debounce wait. `None` when nothing waits.
    fn deadline(&self) -> Option<DateTime<Utc>> {
        let debounce = self.settings.as_ref()?.debounce;
        let first = self.first_at?;
        let last = self.last_at?;
        Some((last + debounce).min(first + debounce * CAP))
    }
}

/// The sync scope of a watched tab.
fn tab_scope(tab: &str) -> Option<Scope> {
    match tab {
        "mine" => Some(Scope::Mine),
        "review" | "team" => Some(Scope::Team),
        _ => None,
    }
}

/// True when the pull request is an open pull request of the tab `tab`
/// in the age limit, by the rules of the tabs without their filters.
fn in_tab(data: &AppData, tab: &str, pr: &PullRequest) -> bool {
    let head = &pr.head.value;
    let scope = data.scopes.get(&pr.key);
    let rule = match tab {
        "mine" => scope == Some(&Scope::Mine),
        "review" => scope == Some(&Scope::Team) && head.review_requested,
        "team" => scope == Some(&Scope::Team) && !head.review_requested,
        _ => false,
    };
    rule && head.state == PrState::Open && data.filters.matches(pr, data.now)
}

/// The ready set of the tab `tab`.
fn ready_set(data: &AppData, tab: &str) -> BTreeSet<PrKey> {
    data.prs
        .values()
        .filter(|pr| in_tab(data, tab, pr) && is_ready(pr))
        .map(|pr| pr.key.clone())
        .collect()
}

/// Compares the ready set of each watched tab with the last one. A pull
/// request that entered it and that chaoshub did not announce waits for
/// the notice. `synced` holds the scopes that had a sync before the
/// change: for the other scopes, the ready set is only the new baseline.
pub(crate) fn detect(data: &mut AppData, synced: &HashSet<Scope>) {
    let Some(settings) = data.notify.settings.clone() else {
        return;
    };
    let now = data.now;
    for tab in &settings.tabs {
        let current = ready_set(data, tab);
        let baseline = tab_scope(tab).is_none_or(|scope| !synced.contains(&scope));
        let notify = &mut data.notify;
        if let Some(before) = notify.ready.get(tab)
            && !baseline
        {
            for key in current.difference(before) {
                if notify.announced.insert(key.clone()) {
                    notify.pending.push((tab.clone(), key.clone()));
                    notify.first_at.get_or_insert(now);
                    notify.last_at = Some(now);
                }
            }
        }
        notify.ready.insert(tab.clone(), current);
    }
}

/// The notice of the pending pull requests when the debounce wait ended
/// at `now`, and the write of the announced pull requests into
/// `ui_state`. The notice text is also the return value, for the status
/// bar. `None` while the wait runs or nothing waits.
pub(crate) fn announce(data: &mut AppData, now: DateTime<Utc>) -> Option<(String, Cmd)> {
    if data.notify.deadline().is_none_or(|deadline| now < deadline) {
        return None;
    }
    let pending = std::mem::take(&mut data.notify.pending);
    data.notify.first_at = None;
    data.notify.last_at = None;
    let msg = notice_text(data, &pending);
    let notify = &mut data.notify;
    notify.seq += 1;
    notify.notice = Some(Notice {
        seq: notify.seq,
        title: TITLE.to_owned(),
        msg: msg.clone(),
    });
    Some((msg, persist_announced(data)))
}

/// The text of a notice. One pull request: `#123 <title> (<author>)`.
/// More: the count of each tab in tab bar order, for example `3 new PRs
/// in team` or `2 new PRs in review, 1 in team`.
fn notice_text(data: &AppData, pending: &[(String, PrKey)]) -> String {
    if let [(_, key)] = pending
        && let Some(pr) = data.prs.get(key)
    {
        let head = &pr.head.value;
        return format!("#{} {} ({})", key.number, head.title, head.author);
    }
    let counts: Vec<(&str, usize)> = NOTIFY_TABS
        .iter()
        .map(|&tab| (tab, pending.iter().filter(|(t, _)| t == tab).count()))
        .filter(|(_, count)| *count > 0)
        .collect();
    counts
        .iter()
        .enumerate()
        .map(|(i, (tab, count))| {
            if i > 0 {
                format!("{count} in {tab}")
            } else if *count == 1 {
                format!("1 new PR in {tab}")
            } else {
                format!("{count} new PRs in {tab}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Drops the announced pull requests that are closed, gone or past the
/// age limit, and returns the write of the others into `ui_state`.
fn persist_announced(data: &mut AppData) -> Cmd {
    let keep: BTreeSet<PrKey> = data
        .notify
        .announced
        .iter()
        .filter(|key| {
            data.prs.get(key).is_some_and(|pr| {
                pr.head.value.state == PrState::Open && data.filters.matches(pr, data.now)
            })
        })
        .cloned()
        .collect();
    data.notify.announced = keep;
    let keys: Vec<&PrKey> = data.notify.announced.iter().collect();
    Cmd::Persist(Persist::UiState {
        key: ANNOUNCED_KEY.to_owned(),
        value: serde_json::to_string(&keys).unwrap_or_default(),
    })
}

/// Clears the notice. True when one showed.
pub(crate) fn dismiss(data: &mut AppData) -> bool {
    data.notify.notice.take().is_some()
}

/// The pull requests of the watched tabs whose CI the CI worker polls in
/// the background: open, not drafts, in the age limit, CI not green, and
/// not announced. Such a pull request becomes new when its CI turns
/// green, and check runs do not change `updatedAt`, so the list sync does
/// not see it soon. Empty while the notification is off.
pub(crate) fn ci_watch(data: &AppData) -> Vec<PrRef> {
    let Some(settings) = &data.notify.settings else {
        return Vec::new();
    };
    data.prs
        .values()
        .filter(|pr| settings.tabs.iter().any(|tab| in_tab(data, tab, pr)))
        .filter(|pr| !pr.head.value.is_draft && pr.ci_state.value != CiState::Success)
        .filter(|pr| !data.notify.announced.contains(&pr.key))
        .map(pr_ref)
        .collect()
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::app::state::AppState;
    use crate::app::testdata;
    use crate::bus::Size;
    use crate::domain::test_time;
    use crate::store::Snapshot;

    const SIZE: Size = Size {
        width: 100,
        height: 30,
    };

    fn at(secs: i64) -> DateTime<Utc> {
        test_time() + TimeDelta::seconds(secs)
    }

    /// A state with the notification on for `tabs`, with a debounce of
    /// `debounce` seconds, at `at(0)`.
    fn state(tabs: &[&str], debounce: i64) -> AppState {
        let mut state = AppState::new(SIZE);
        state.data.now = at(0);
        state.data.notify.settings = Some(NotifySettings {
            tabs: tabs.iter().map(|&tab| tab.to_owned()).collect(),
            debounce: TimeDelta::seconds(debounce),
        });
        state
    }

    /// A ready pull request of `alice` in the Team tab.
    fn team_pr(number: u64, title: &str) -> PullRequest {
        let mut pr = testdata::pr("chaoshub", number, title);
        pr.head.value.author = "alice".into();
        pr.ci_state.value = CiState::Success;
        pr
    }

    /// A ready pull request of `bob` that requests the review of the user.
    fn review_pr(number: u64, title: &str) -> PullRequest {
        let mut pr = team_pr(number, title);
        pr.head.value.author = "bob".into();
        pr.head.value.review_requested = true;
        pr
    }

    fn put(state: &mut AppState, scope: Scope, pr: PullRequest) {
        state.data.scopes.insert(pr.key.clone(), scope);
        state.data.prs.insert(pr.key.clone(), pr);
    }

    fn both() -> HashSet<Scope> {
        HashSet::from([Scope::Mine, Scope::Team])
    }

    /// Sets the time to `at(secs)` and runs `detect` after a sync of both
    /// scopes.
    fn detect_at(state: &mut AppState, secs: i64) {
        state.data.now = at(secs);
        detect(&mut state.data, &both());
    }

    /// The notice text at `at(secs)`, if the wait ended.
    fn announce_at(state: &mut AppState, secs: i64) -> Option<String> {
        announce(&mut state.data, at(secs)).map(|(text, _)| text)
    }

    /// A state after the first sync, with the ready pull request #1 in
    /// the Team tab as the baseline.
    fn synced(tabs: &[&str], debounce: i64) -> AppState {
        let mut state = state(tabs, debounce);
        put(&mut state, Scope::Team, team_pr(1, "Old"));
        detect(&mut state.data, &HashSet::new());
        state
    }

    #[test]
    fn the_first_sync_only_sets_the_baseline() {
        let mut state = state(&["team"], 0);
        put(&mut state, Scope::Team, team_pr(1, "Cached"));
        detect(&mut state.data, &HashSet::new());
        put(&mut state, Scope::Team, team_pr(2, "First sync"));
        detect(&mut state.data, &HashSet::from([Scope::Mine]));
        assert_eq!(announce_at(&mut state, 3600), None);
        assert!(state.data.notify.announced.is_empty());
        assert_eq!(state.data.notify.notice, None);
    }

    #[test]
    fn a_new_ready_pull_request_is_announced() {
        let mut state = synced(&["team"], 0);
        put(&mut state, Scope::Team, team_pr(2, "Add the thing"));
        detect_at(&mut state, 0);
        assert_eq!(
            announce_at(&mut state, 0).as_deref(),
            Some("#2 Add the thing (alice)")
        );
        assert_eq!(
            state.data.notify.notice,
            Some(Notice {
                seq: 1,
                title: "chaoshub".into(),
                msg: "#2 Add the thing (alice)".into(),
            })
        );
        assert_eq!(announce_at(&mut state, 60), None, "one notice only");
    }

    #[test]
    fn a_pull_request_that_leaves_draft_is_announced() {
        let mut state = synced(&["team"], 0);
        let mut draft = team_pr(2, "Draft");
        draft.head.value.is_draft = true;
        put(&mut state, Scope::Team, draft.clone());
        detect_at(&mut state, 0);
        assert_eq!(announce_at(&mut state, 0), None);
        draft.head.value.is_draft = false;
        put(&mut state, Scope::Team, draft);
        detect_at(&mut state, 10);
        assert_eq!(
            announce_at(&mut state, 10).as_deref(),
            Some("#2 Draft (alice)")
        );
    }

    #[test]
    fn a_pull_request_that_turns_green_is_announced() {
        let mut state = synced(&["team"], 0);
        let mut pending = team_pr(2, "Pending");
        pending.ci_state.value = CiState::Pending;
        put(&mut state, Scope::Team, pending.clone());
        detect_at(&mut state, 0);
        assert_eq!(announce_at(&mut state, 0), None);
        pending.ci_state.value = CiState::Success;
        put(&mut state, Scope::Team, pending);
        detect_at(&mut state, 10);
        assert_eq!(
            announce_at(&mut state, 10).as_deref(),
            Some("#2 Pending (alice)")
        );
    }

    #[test]
    fn an_announced_pull_request_is_not_announced_again() {
        let mut state = synced(&["team"], 0);
        let mut pr = team_pr(2, "Flaky");
        put(&mut state, Scope::Team, pr.clone());
        detect_at(&mut state, 0);
        assert!(announce_at(&mut state, 0).is_some());
        pr.ci_state.value = CiState::Failure;
        put(&mut state, Scope::Team, pr.clone());
        detect_at(&mut state, 10);
        pr.ci_state.value = CiState::Success;
        put(&mut state, Scope::Team, pr);
        detect_at(&mut state, 20);
        assert_eq!(announce_at(&mut state, 20), None);
    }

    #[test]
    fn an_announced_pull_request_stays_announced_after_a_restart() {
        let mut first = synced(&["team"], 0);
        put(&mut first, Scope::Team, team_pr(2, "Announced"));
        detect_at(&mut first, 0);
        let Some((_, Cmd::Persist(Persist::UiState { key, value }))) =
            announce(&mut first.data, at(0))
        else {
            panic!("expected the write of the announced pull requests");
        };
        assert_eq!(key, ANNOUNCED_KEY);

        // The next session: the cache holds #2 with pending CI.
        let mut pending = team_pr(2, "Announced");
        pending.ci_state.value = CiState::Pending;
        let snapshot = Snapshot {
            prs: vec![(Scope::Team, pending.clone())],
            ui_state: [(key, value)].into_iter().collect(),
            ..Snapshot::default()
        };
        let mut state = state(&["team"], 0);
        state.load(snapshot, Vec::new());
        assert!(
            state
                .data
                .notify
                .announced
                .contains(&testdata::pr_key("chaoshub", 2))
        );
        detect(&mut state.data, &HashSet::new());
        pending.ci_state.value = CiState::Success;
        put(&mut state, Scope::Team, pending);
        detect_at(&mut state, 10);
        assert_eq!(announce_at(&mut state, 10), None);
    }

    #[test]
    fn a_bad_announced_value_gives_none() {
        let mut notify = NotifyState::default();
        notify.load_announced("not json");
        assert!(notify.announced.is_empty());
    }

    #[test]
    fn pull_requests_outside_the_watched_tabs_are_ignored() {
        let mut state = synced(&["team"], 0);
        put(&mut state, Scope::Team, review_pr(2, "Review me"));
        put(&mut state, Scope::Mine, team_pr(3, "Mine"));
        detect_at(&mut state, 0);
        assert_eq!(announce_at(&mut state, 0), None);
    }

    #[test]
    fn pull_requests_outside_the_age_limit_are_ignored() {
        let mut state = synced(&["team"], 0);
        state.data.filters.max_age = Some(TimeDelta::days(1));
        let mut old = team_pr(2, "Old");
        old.head.value.created_at = Some(at(0) - TimeDelta::days(2));
        put(&mut state, Scope::Team, old);
        detect_at(&mut state, 0);
        assert_eq!(announce_at(&mut state, 0), None);
    }

    #[test]
    fn the_filters_of_the_tabs_are_ignored() {
        let mut state = synced(&["team"], 0);
        for ts in &mut state.ui.tabs {
            ts.author = Some("nobody".into());
            ts.filter = "no match".into();
            ts.show_hidden = true;
        }
        put(&mut state, Scope::Team, team_pr(2, "Hidden in the list"));
        detect_at(&mut state, 0);
        assert_eq!(
            announce_at(&mut state, 0).as_deref(),
            Some("#2 Hidden in the list (alice)")
        );
    }

    #[test]
    fn each_new_pull_request_starts_the_wait_again() {
        let mut state = synced(&["team"], 15);
        put(&mut state, Scope::Team, team_pr(2, "A"));
        detect_at(&mut state, 0);
        assert_eq!(announce_at(&mut state, 14), None);
        put(&mut state, Scope::Team, team_pr(3, "B"));
        detect_at(&mut state, 10);
        assert_eq!(announce_at(&mut state, 15), None);
        assert_eq!(announce_at(&mut state, 24), None);
        assert_eq!(
            announce_at(&mut state, 25).as_deref(),
            Some("2 new PRs in team")
        );
    }

    #[test]
    fn the_wait_ends_after_5_debounce_times() {
        let mut state = synced(&["team"], 15);
        for i in 0..8_u64 {
            let secs = i64::try_from(i * 10).unwrap();
            put(&mut state, Scope::Team, team_pr(10 + i, "steady"));
            detect_at(&mut state, secs);
            if secs < 75 {
                assert_eq!(announce_at(&mut state, secs), None, "at {secs} s");
            }
        }
        // The first one came at 0 s: the wait ends at 75 s, although the
        // last one came at 70 s.
        assert_eq!(
            announce_at(&mut state, 75).as_deref(),
            Some("8 new PRs in team")
        );
    }

    #[test]
    fn a_debounce_of_zero_announces_at_once() {
        let mut state = synced(&["team"], 0);
        put(&mut state, Scope::Team, team_pr(2, "Now"));
        detect_at(&mut state, 5);
        assert!(announce_at(&mut state, 5).is_some());
    }

    #[test]
    fn the_notice_counts_each_tab() {
        let mut state = synced(&["team", "review"], 0);
        put(&mut state, Scope::Team, team_pr(2, "A"));
        put(&mut state, Scope::Team, review_pr(3, "B"));
        put(&mut state, Scope::Team, review_pr(4, "C"));
        detect_at(&mut state, 0);
        assert_eq!(
            announce_at(&mut state, 0).as_deref(),
            Some("2 new PRs in review, 1 in team")
        );
        put(&mut state, Scope::Team, review_pr(5, "D"));
        put(&mut state, Scope::Team, team_pr(6, "E"));
        detect_at(&mut state, 10);
        assert_eq!(
            announce_at(&mut state, 10).as_deref(),
            Some("1 new PR in review, 1 in team")
        );
        assert_eq!(state.data.notify.notice.as_ref().map(|n| n.seq), Some(2));
    }

    #[test]
    fn the_write_drops_closed_and_old_pull_requests() {
        let mut state = synced(&["team"], 0);
        state.data.notify.announced = [
            testdata::pr_key("chaoshub", 1),
            testdata::pr_key("chaoshub", 8),
            testdata::pr_key("chaoshub", 9),
        ]
        .into_iter()
        .collect();
        let mut closed = team_pr(8, "Closed");
        closed.head.value.state = PrState::Merged;
        put(&mut state, Scope::Team, closed);
        put(&mut state, Scope::Team, team_pr(2, "New"));
        detect_at(&mut state, 0);
        let Some((_, Cmd::Persist(Persist::UiState { value, .. }))) =
            announce(&mut state.data, at(0))
        else {
            panic!("expected the write of the announced pull requests");
        };
        let keys: Vec<PrKey> = serde_json::from_str(&value).unwrap();
        assert_eq!(
            keys,
            [
                testdata::pr_key("chaoshub", 1),
                testdata::pr_key("chaoshub", 2)
            ]
        );
    }

    #[test]
    fn dismiss_clears_the_notice() {
        let mut state = synced(&["team"], 0);
        assert!(!dismiss(&mut state.data));
        put(&mut state, Scope::Team, team_pr(2, "A"));
        detect_at(&mut state, 0);
        announce_at(&mut state, 0);
        assert!(dismiss(&mut state.data));
        assert_eq!(state.data.notify.notice, None);
    }

    #[test]
    fn ci_watch_holds_the_pull_requests_that_can_turn_green() {
        let mut state = synced(&["team", "review"], 0);
        let with_ci = |mut pr: PullRequest, ci| {
            pr.ci_state.value = ci;
            pr
        };
        put(
            &mut state,
            Scope::Team,
            with_ci(team_pr(2, "Pending"), CiState::Pending),
        );
        put(
            &mut state,
            Scope::Team,
            with_ci(review_pr(3, "Failing"), CiState::Failure),
        );
        put(
            &mut state,
            Scope::Team,
            with_ci(team_pr(4, "No checks"), CiState::None),
        );
        let mut draft = with_ci(team_pr(5, "Draft"), CiState::Pending);
        draft.head.value.is_draft = true;
        put(&mut state, Scope::Team, draft);
        let mut closed = with_ci(team_pr(6, "Closed"), CiState::Pending);
        closed.head.value.state = PrState::Closed;
        put(&mut state, Scope::Team, closed);
        put(
            &mut state,
            Scope::Mine,
            with_ci(team_pr(7, "Mine"), CiState::Pending),
        );
        put(
            &mut state,
            Scope::Team,
            with_ci(team_pr(8, "Announced"), CiState::Pending),
        );
        state
            .data
            .notify
            .announced
            .insert(testdata::pr_key("chaoshub", 8));
        let numbers: Vec<u64> = ci_watch(&state.data)
            .into_iter()
            .map(|pr| pr.key.number)
            .collect();
        assert_eq!(numbers, [2, 3, 4], "not green, not #1 (green)");
        state.data.notify.settings = None;
        assert_eq!(ci_watch(&state.data), []);
    }

    #[test]
    fn nothing_happens_while_the_notification_is_off() {
        let mut state = AppState::new(SIZE);
        put(&mut state, Scope::Team, team_pr(1, "A"));
        detect(&mut state.data, &both());
        put(&mut state, Scope::Team, team_pr(2, "B"));
        detect(&mut state.data, &both());
        assert_eq!(announce_at(&mut state, 60), None);
    }
}
