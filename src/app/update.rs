//! The `update` function: applies a `Msg` and returns commands.
//!
//! There is one function per message family. Commands are plain data, so
//! the tests compare them with `assert_eq!`.

use std::collections::BTreeSet;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

use crate::app::frame;
use crate::app::keymap::{self, Keymap};
use crate::app::layout::layout;
use crate::app::models::CellStyle;
use crate::app::nav::{self, Motion};
use crate::app::state::{AppData, AppState, Failure, error_text};
use crate::app::tabs::{KeyCx, TABS, Tab, pr_ref};
use crate::app::term::term_status;
use crate::bus::{
    ActionEvent, Cmd, KeyChord, KeyEvent, KeyEventKind, Msg, PrRef, Scope, Size, SyncError,
    TargetKey, UiEvent,
};
use crate::domain::keys::PrKey;
use crate::domain::observed::{Observed, apply_newer};
use crate::domain::pr::PullRequest;
use crate::domain::repo::Repo;

/// Applies one message to the state, with the tabs of `TABS`.
pub(crate) fn update(state: &mut AppState, keymap: &Keymap, msg: Msg) -> Vec<Cmd> {
    update_with(state, keymap, &TABS, msg)
}

/// Applies one message to the state, with the tabs of `tabs`.
#[allow(
    clippy::needless_pass_by_value,
    reason = "the data messages of later phases move their payload into the state"
)]
pub(crate) fn update_with(
    state: &mut AppState,
    keymap: &Keymap,
    tabs: &[Box<dyn Tab>],
    msg: Msg,
) -> Vec<Cmd> {
    match msg {
        Msg::Key(event) => key(state, keymap, tabs, &event),
        Msg::Resize(size) => resize(state, size),
        Msg::Tick => tick(state, tabs, Utc::now()),
        Msg::TransitionsDone => transitions_done(state),
        Msg::Signal => quit(),
        Msg::PrsSynced {
            scope,
            prs,
            gone,
            started_at,
        } => {
            let mut cmds = prs_synced(state, scope, prs, gone, started_at);
            cmds.extend(refresh_interest(state, tabs));
            cmds
        }
        Msg::PrRefreshed(pr) => {
            if pr_refreshed(state, *pr) {
                refresh_interest(state, tabs).into_iter().collect()
            } else {
                Vec::new()
            }
        }
        Msg::SyncFailed { source, scope, err } => sync_failed(state, source, scope, &err),
        Msg::WorkerDied { name, restart_in } => worker_died(state, name, restart_in),
        Msg::Rate(info) => {
            state.data.sync.rate = Some(info);
            Vec::new()
        }
        Msg::ActionEvent {
            key,
            event: ActionEvent::Done(_),
            ..
        }
        | Msg::PaneEnded { key, .. } => follow_up(&state.data, &key),
        _ => {
            tracing::trace!("message not handled yet");
            Vec::new()
        }
    }
}

/// A key press: the quit keys quit, every other key goes to the active
/// tab. After a handled key, the generation moves when the tab says so,
/// the UI events queue up, and a changed `Interest` goes to the workers.
fn key(state: &mut AppState, keymap: &Keymap, tabs: &[Box<dyn Tab>], event: &KeyEvent) -> Vec<Cmd> {
    if event.kind == KeyEventKind::Release {
        return Vec::new();
    }
    if state.data.sync.failure.take().is_some() {
        state.bump();
    }
    let chord = keymap::chord(event);
    if keymap.matches("quit", chord) {
        return quit();
    }
    if keymap.matches("refresh", chord) {
        let mut cmds = refresh(state, tabs);
        cmds.extend(refresh_interest(state, tabs));
        return cmds;
    }
    if let Some(motion) = motion(keymap, chord) {
        navigate(state, tabs, motion);
        return refresh_interest(state, tabs).into_iter().collect();
    }
    let index = state.ui.active;
    let (Some(tab), Some(ts)) = (tabs.get(index), state.ui.tabs.get_mut(index)) else {
        return Vec::new();
    };
    let areas = layout(state.data.size);
    let cx = KeyCx {
        data: &state.data,
        target: None,
        areas: &areas,
    };
    let Some(handled) = tab.handle_key(chord, &cx, ts) else {
        return Vec::new();
    };
    if handled.changed {
        state.bump();
    }
    state.ui.ui_events.extend(handled.events);
    let mut cmds = handled.cmds;
    cmds.extend(refresh_interest(state, tabs));
    cmds
}

/// The navigation binding of a chord, if any.
fn motion(keymap: &Keymap, chord: KeyChord) -> Option<Motion> {
    [
        ("down", Motion::Down),
        ("up", Motion::Up),
        ("top", Motion::Top),
        ("bottom", Motion::Bottom),
        ("page_down", Motion::PageDown),
        ("page_up", Motion::PageUp),
    ]
    .into_iter()
    .find(|(name, _)| keymap.matches(name, chord))
    .map(|(_, motion)| motion)
}

/// Moves the cursor of the active tab. A move that scrolls the list pushes
/// `UiEvent::ScrollTo`; a move inside the visible rows is immediate.
fn navigate(state: &mut AppState, tabs: &[Box<dyn Tab>], motion: Motion) {
    let Some(tab) = tabs.get(state.ui.active) else {
        return;
    };
    let height = usize::from(frame::list_size(state).height);
    let model = frame::active_list(state, tabs);
    let Some(cursor) = nav::step(&model.rows, model.cursor, motion, height) else {
        return;
    };
    let scroll = nav::keep_visible(model.rows.len(), cursor, model.scroll, height);
    let Some(ts) = state.ui.tabs.get_mut(state.ui.active) else {
        return;
    };
    ts.cursor = cursor;
    ts.scroll = scroll;
    ts.selected = model.rows.get(cursor).map(|row| row.item.clone());
    if cursor == model.cursor && scroll == model.scroll {
        return;
    }
    if scroll != model.scroll {
        state.ui.ui_events.push(UiEvent::ScrollTo {
            tab: tab.id(),
            from: model.scroll,
            to: scroll,
        });
    }
    state.bump();
}

/// `Cmd::SetInterest` when the interest of the active tab differs from
/// the interest that the workers have.
fn refresh_interest(state: &mut AppState, tabs: &[Box<dyn Tab>]) -> Option<Cmd> {
    let index = state.ui.active;
    let (tab, ts) = (tabs.get(index)?, state.ui.tabs.get(index)?);
    let areas = layout(state.data.size);
    let cx = KeyCx {
        data: &state.data,
        target: None,
        areas: &areas,
    };
    let interest = tab.interest(&cx, ts);
    if state.ui.interest.as_ref() == Some(&interest) {
        return None;
    }
    state.ui.interest = Some(interest.clone());
    Some(Cmd::SetInterest(interest))
}

/// A sync of one scope: the pull requests go into the data under the
/// freshness rule, and the gone candidates of a full sync leave it. Each
/// repo of the result gets a jj scan.
fn prs_synced(
    state: &mut AppState,
    scope: Scope,
    prs: Vec<PullRequest>,
    gone: Option<Vec<PrKey>>,
    started_at: DateTime<Utc>,
) -> Vec<Cmd> {
    let repos: BTreeSet<_> = prs.iter().map(|pr| pr.key.repo.clone()).collect();
    let mut changed = false;
    for pr in prs {
        state.data.scopes.insert(pr.key.clone(), scope);
        changed |= merge_pr(&mut state.data, pr);
    }
    for key in gone.into_iter().flatten() {
        changed |= drop_gone(&mut state.data, &key, started_at);
    }
    state.data.synced.insert(scope, started_at);
    state.data.sync.pending.remove(&scope);
    state.data.sync.failure = None;
    if changed {
        state.data.revision += 1;
    }
    state.bump();
    repos.into_iter().map(Cmd::ScanJj).collect()
}

/// A pull request that the sync worker read again by id. True when it
/// changed the data.
fn pr_refreshed(state: &mut AppState, pr: PullRequest) -> bool {
    let data = &mut state.data;
    if !data.scopes.contains_key(&pr.key) {
        let mine = data.session.login.as_deref() == Some(pr.head.value.author.as_str());
        let scope = if mine { Scope::Mine } else { Scope::Team };
        data.scopes.insert(pr.key.clone(), scope);
    }
    if !merge_pr(data, pr) {
        return false;
    }
    data.revision += 1;
    state.bump();
    true
}

/// Merges one pull request into the data. Each field group follows the
/// freshness rule on its own. The merge settings of the repo come along.
/// True when a value changed.
fn merge_pr(data: &mut AppData, incoming: PullRequest) -> bool {
    fold_repo_settings(data, &incoming);
    let Some(current) = data.prs.get_mut(&incoming.key) else {
        data.prs.insert(incoming.key.clone(), incoming);
        return true;
    };
    let PullRequest {
        key,
        node_id,
        head,
        ci_state,
        checks,
    } = incoming;
    if head.observed_at >= current.head.observed_at {
        current.node_id = node_id;
    }
    let mut changed = apply(&mut current.head, head, &key, "head");
    changed |= apply(&mut current.ci_state, ci_state, &key, "ci state");
    changed |= apply(&mut current.checks, checks, &key, "checks");
    changed
}

/// Applies `incoming` when it is not older. True when the value changed.
/// Older data is dropped and logged.
fn apply<T: PartialEq>(
    current: &mut Observed<T>,
    incoming: Observed<T>,
    key: &PrKey,
    group: &str,
) -> bool {
    let same = current.value == incoming.value;
    if apply_newer(current, incoming) {
        !same
    } else {
        tracing::debug!(?key, group, "older data dropped");
        false
    }
}

/// Folds the merge settings of the head of `pr` into the repo of `pr`. A
/// known default branch stays when the new settings have none.
fn fold_repo_settings(data: &mut AppData, pr: &PullRequest) {
    let key = &pr.key.repo;
    let repo = data.repos.entry(key.clone()).or_insert_with(|| Repo {
        key: key.clone(),
        path: None,
        settings: None,
    });
    let mut settings = pr.head.value.repo_settings.clone();
    if settings.default_branch.is_none() {
        settings.default_branch = repo
            .settings
            .as_ref()
            .and_then(|old| old.default_branch.clone());
    }
    repo.settings = Some(settings);
}

/// The gone rule: a candidate leaves only when its head was observed
/// before the full sync started. True when it left.
fn drop_gone(data: &mut AppData, key: &PrKey, started_at: DateTime<Utc>) -> bool {
    let Some(pr) = data.prs.get(key) else {
        return false;
    };
    if pr.head.observed_at > started_at {
        return false;
    }
    data.prs.remove(key);
    data.scopes.remove(key);
    data.details.remove(key);
    true
}

/// The follow-up rule after an action or a pane: scan the repo of the
/// target, and refresh the stack group of a pull request (or the pull
/// request alone when it has no stack).
fn follow_up(data: &AppData, key: &TargetKey) -> Vec<Cmd> {
    match key {
        TargetKey::Local { repo, .. } => vec![Cmd::ScanJj(repo.clone())],
        TargetKey::Pr(pr) => {
            let mut cmds = vec![Cmd::ScanJj(pr.repo.clone())];
            let refs = stack_group_refs(data, pr);
            if !refs.is_empty() {
                cmds.push(Cmd::RefreshPr(refs));
            }
            cmds
        }
    }
}

/// The refs of the loaded pull requests in the stack of `key`, position 1
/// first, or of `key` alone when it has no stack. Empty when `key` is not
/// loaded.
fn stack_group_refs(data: &AppData, key: &PrKey) -> Vec<PrRef> {
    let Some(pr) = data.prs.get(key) else {
        return Vec::new();
    };
    let Some(stack) = &pr.head.value.stack else {
        return vec![pr_ref(pr)];
    };
    let mut group: Vec<&PullRequest> = data
        .prs
        .values()
        .filter(|other| {
            other
                .head
                .value
                .stack
                .as_ref()
                .is_some_and(|s| s.id == stack.id)
        })
        .collect();
    group.sort_by_key(|other| other.head.value.stack.as_ref().map(|s| s.position));
    group.into_iter().map(pr_ref).collect()
}

/// A new terminal size.
fn resize(state: &mut AppState, size: Size) -> Vec<Cmd> {
    state.data.size = size;
    state.bump();
    Vec::new()
}

/// The deadline of the event loop at `now`. An expired status message
/// leaves the status bar and starts its fade. Else the generation moves
/// only when a rendered string changed since the last frame: at a new
/// minute (ages have minute granularity), or when the status bar or the
/// terminal status changed.
fn tick(state: &mut AppState, tabs: &[Box<dyn Tab>], now: DateTime<Utc>) -> Vec<Cmd> {
    let minute = |time: DateTime<Utc>| time.timestamp().div_euclid(60);
    let new_minute = minute(now) != minute(state.data.now);
    state.data.now = now;
    if state
        .ui
        .status
        .as_ref()
        .is_some_and(|msg| msg.expires <= now)
    {
        state.ui.status = None;
        state.ui.ui_events.push(UiEvent::StatusExpired);
        state.bump();
        return Vec::new();
    }
    let Some((status, term)) = frame::built_status(state) else {
        return Vec::new();
    };
    if new_minute || *status != frame::status_model(state, tabs) || *term != term_status(state) {
        state.bump();
    }
    Vec::new()
}

/// How long an info message stays in the status bar.
const INFO_TTL: TimeDelta = TimeDelta::seconds(5);
/// How long an error message stays in the status bar.
const ERROR_TTL: TimeDelta = TimeDelta::seconds(10);

/// The `refresh` key: a full sync of every scope that feeds a tab.
fn refresh(state: &mut AppState, tabs: &[Box<dyn Tab>]) -> Vec<Cmd> {
    let mut cmds = Vec::new();
    for scope in tabs.iter().filter_map(|tab| tab.scope()) {
        if state.data.sync.pending.insert(scope) {
            cmds.push(Cmd::SyncNow(scope));
        }
    }
    state.bump();
    cmds
}

/// A worker failed. A sync failure and a rate limit become the failure
/// of the terminal status; the other workers only show a message.
fn sync_failed(
    state: &mut AppState,
    source: &'static str,
    scope: Option<Scope>,
    err: &SyncError,
) -> Vec<Cmd> {
    let pending = &mut state.data.sync.pending;
    match scope {
        Some(scope) => {
            pending.remove(&scope);
        }
        None if source == "sync" => pending.clear(),
        None => {}
    }
    let reason = error_text(err);
    let rate_limited_until = match err {
        SyncError::RateLimited { until } => Some(*until),
        _ => None,
    };
    if source != "sync" && rate_limited_until.is_none() {
        state.message(
            format!("{source} failed: {reason}"),
            CellStyle::Warn,
            INFO_TTL,
        );
        return Vec::new();
    }
    let (reason, style) = match rate_limited_until {
        Some(_) => (reason, CellStyle::Warn),
        None => (format!("sync failed: {reason}"), CellStyle::Danger),
    };
    state.data.sync.failure = Some(Failure {
        source,
        reason: reason.clone(),
        rate_limited_until,
        at: state.data.now,
    });
    state.message(reason, style, ERROR_TTL);
    Vec::new()
}

/// The supervisor restarts a worker that panicked.
fn worker_died(state: &mut AppState, name: &'static str, restart_in: Duration) -> Vec<Cmd> {
    state.data.sync.failure = Some(Failure {
        source: name,
        reason: format!("{name} worker died"),
        rate_limited_until: None,
        at: state.data.now,
    });
    let text = format!("{name} worker restarting in {}s", restart_in.as_secs());
    state.message(text, CellStyle::Warn, ERROR_TTL);
    Vec::new()
}

/// The transitions are done: the previous list of a tab slide and the
/// overlays that slid out go away.
fn transitions_done(state: &mut AppState) -> Vec<Cmd> {
    let had_prev_list = state.ui.prev_list.take().is_some();
    let overlays = state.ui.overlays.len();
    state.ui.overlays.retain(|overlay| !overlay.closing);
    if had_prev_list || state.ui.overlays.len() != overlays {
        state.bump();
    }
    Vec::new()
}

/// Quit, from a quit key or a signal.
fn quit() -> Vec<Cmd> {
    vec![Cmd::Quit]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::models::{Direction, ListModel};
    use crate::app::state::Overlay;
    use crate::app::tabs::Handled;
    use crate::app::tabs::fake::FakeTab;
    use crate::app::testdata;
    use crate::bus::{Interest, KeyCode, KeyModifiers, OverlayKind, RateInfo};
    use crate::domain::test_time;

    const SIZE: Size = Size {
        width: 100,
        height: 30,
    };

    /// A state with one fake tab that handles every key with `handled`.
    fn setup(handled: Option<Handled>) -> (Vec<Box<dyn Tab>>, AppState) {
        let mut tab = FakeTab::new("one");
        tab.handled = handled;
        let tabs: Vec<Box<dyn Tab>> = vec![Box::new(tab)];
        let state = AppState::with_tabs(SIZE, &tabs);
        (tabs, state)
    }

    fn press(code: KeyCode, mods: KeyModifiers) -> Msg {
        Msg::Key(KeyEvent::new(code, mods))
    }

    fn run(state: &mut AppState, tabs: &[Box<dyn Tab>], msg: Msg) -> Vec<Cmd> {
        update_with(state, &Keymap::default(), tabs, msg)
    }

    fn empty_list() -> ListModel {
        ListModel {
            columns: Vec::new(),
            rows: Vec::new(),
            cursor: 0,
            scroll: 0,
        }
    }

    #[test]
    fn q_and_ctrl_c_quit() {
        let (tabs, mut state) = setup(None);
        let q = press(KeyCode::Char('q'), KeyModifiers::NONE);
        let ctrl_c = press(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(run(&mut state, &tabs, q), vec![Cmd::Quit]);
        assert_eq!(run(&mut state, &tabs, ctrl_c), vec![Cmd::Quit]);
    }

    #[test]
    fn a_signal_quits() {
        let (tabs, mut state) = setup(None);
        assert_eq!(run(&mut state, &tabs, Msg::Signal), vec![Cmd::Quit]);
    }

    #[test]
    fn a_key_release_does_nothing() {
        let (tabs, mut state) = setup(None);
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        assert_eq!(run(&mut state, &tabs, Msg::Key(release)), vec![]);
    }

    #[test]
    fn a_key_goes_to_the_active_tab() {
        let handled = Handled {
            cmds: vec![Cmd::SyncNow(Scope::Mine)],
            events: vec![UiEvent::StatusExpired],
            changed: true,
        };
        let (tabs, mut state) = setup(Some(handled));
        let j = press(KeyCode::Char('z'), KeyModifiers::NONE);
        let interest = Interest {
            tab: "one",
            scope: None,
            wants_local: true,
            selected: None,
            visible: Vec::new(),
        };
        assert_eq!(
            run(&mut state, &tabs, j.clone()),
            vec![Cmd::SyncNow(Scope::Mine), Cmd::SetInterest(interest)]
        );
        assert_eq!(state.generation, 1);
        assert_eq!(state.ui.tabs[0].cursor, 1);
        assert_eq!(state.ui.ui_events, vec![UiEvent::StatusExpired]);
        assert_eq!(
            run(&mut state, &tabs, j),
            vec![Cmd::SyncNow(Scope::Mine)],
            "an unchanged interest is not sent again"
        );
        assert_eq!(state.generation, 2);
    }

    #[test]
    fn an_unhandled_key_changes_nothing() {
        let (tabs, mut state) = setup(None);
        let j = press(KeyCode::Char('z'), KeyModifiers::NONE);
        assert_eq!(run(&mut state, &tabs, j), vec![]);
        assert_eq!(state.generation, 0);
    }

    #[test]
    fn resize_stores_the_size_and_moves_the_generation() {
        let (tabs, mut state) = setup(None);
        let size = Size {
            width: 160,
            height: 50,
        };
        assert_eq!(run(&mut state, &tabs, Msg::Resize(size)), vec![]);
        assert_eq!(state.data.size, size);
        assert_eq!(state.generation, 1);
    }

    #[test]
    fn tick_moves_the_generation_only_when_a_string_changed() {
        let (tabs, mut state) = setup(None);
        state.data.now = at(0);
        assert_eq!(tick(&mut state, &tabs, at(1)), vec![]);
        assert_eq!(state.generation, 0, "no frame was built yet");
        frame::frame_model_with(&mut state, &tabs);
        assert_eq!(tick(&mut state, &tabs, at(2)), vec![]);
        assert_eq!(state.generation, 0, "the same minute");
        state.ui.status = Some(crate::app::state::StatusMsg {
            text: "synced".into(),
            style: CellStyle::Normal,
            expires: at(600),
        });
        assert_eq!(tick(&mut state, &tabs, at(3)), vec![]);
        assert_eq!(state.generation, 1, "the status bar changed");
        frame::frame_model_with(&mut state, &tabs);
        tick(&mut state, &tabs, at(4));
        assert_eq!(state.generation, 1);
        tick(&mut state, &tabs, at(60));
        assert_eq!(state.generation, 2, "a new minute");
    }

    #[test]
    fn an_expired_message_leaves_on_its_tick_with_the_fade_event() {
        let (tabs, mut state) = setup(None);
        state.data.now = at(0);
        state.message("merged #101", CellStyle::Success, TimeDelta::seconds(5));
        let generation = state.generation;
        tick(&mut state, &tabs, at(4));
        assert!(state.ui.status.is_some());
        assert_eq!(state.ui.ui_events, vec![]);
        tick(&mut state, &tabs, at(5));
        assert_eq!(state.ui.status, None);
        assert_eq!(state.ui.ui_events, vec![UiEvent::StatusExpired]);
        assert_eq!(state.generation, generation + 1);
    }

    #[test]
    fn sync_failed_sets_the_failure_until_a_key_or_a_success() {
        let (tabs, mut state) = setup(None);
        state.data.now = at(0);
        state.data.sync.pending.insert(Scope::Mine);
        let failed = Msg::SyncFailed {
            source: "sync",
            scope: Some(Scope::Mine),
            err: SyncError::Offline,
        };
        assert_eq!(run(&mut state, &tabs, failed.clone()), vec![]);
        assert_eq!(state.data.sync.pending.len(), 0);
        let failure = state.data.sync.failure.clone().unwrap();
        assert_eq!(failure.reason, "sync failed: offline");
        assert_eq!(failure.at, at(0));
        let msg = state.ui.status.clone().unwrap();
        assert_eq!(
            (msg.text.as_str(), msg.style),
            ("sync failed: offline", CellStyle::Danger)
        );
        run(
            &mut state,
            &tabs,
            press(KeyCode::Char('j'), KeyModifiers::NONE),
        );
        assert_eq!(state.data.sync.failure, None, "a key press clears it");
        run(&mut state, &tabs, failed);
        run(&mut state, &tabs, synced(Vec::new(), None, 0));
        assert_eq!(state.data.sync.failure, None, "a success clears it");
    }

    #[test]
    fn other_workers_and_rate_limits() {
        let (tabs, mut state) = setup(None);
        let ci = Msg::SyncFailed {
            source: "ci",
            scope: None,
            err: SyncError::Other("502".into()),
        };
        run(&mut state, &tabs, ci);
        assert_eq!(
            state.data.sync.failure, None,
            "a ci failure is a message only"
        );
        assert_eq!(state.ui.status.clone().unwrap().text, "ci failed: 502");
        let until = at(600);
        let limited = Msg::SyncFailed {
            source: "ci",
            scope: None,
            err: SyncError::RateLimited { until },
        };
        run(&mut state, &tabs, limited);
        let failure = state.data.sync.failure.clone().unwrap();
        assert_eq!(failure.rate_limited_until, Some(until));
        assert_eq!(
            failure.reason,
            error_text(&SyncError::RateLimited { until })
        );
        assert!(failure.reason.starts_with("rate limited until "));
        let died = Msg::WorkerDied {
            name: "sync",
            restart_in: Duration::from_secs(2),
        };
        run(&mut state, &tabs, died);
        assert_eq!(
            state.data.sync.failure.clone().unwrap().reason,
            "sync worker died"
        );
        assert_eq!(
            state.ui.status.clone().unwrap().text,
            "sync worker restarting in 2s"
        );
    }

    #[test]
    fn refresh_syncs_every_tab_scope_once() {
        let mine = FakeTab::new("one");
        let tabs: Vec<Box<dyn Tab>> = vec![Box::new(mine)];
        let mut state = AppState::with_tabs(SIZE, &tabs);
        let r = press(KeyCode::Char('r'), KeyModifiers::NONE);
        assert_eq!(
            run(&mut state, &tabs, r.clone()),
            vec![fake_interest()],
            "no scope"
        );
        let real = &TABS;
        let mut state = AppState::new(SIZE);
        let cmds = update(&mut state, &Keymap::default(), r.clone());
        let scopes: Vec<_> = real.iter().filter_map(|tab| tab.scope()).collect();
        let syncs: Vec<_> = cmds
            .iter()
            .filter(|cmd| matches!(cmd, Cmd::SyncNow(_)))
            .cloned()
            .collect();
        assert_eq!(
            syncs,
            scopes.into_iter().map(Cmd::SyncNow).collect::<Vec<_>>()
        );
        let again = update(&mut state, &Keymap::default(), r);
        assert!(
            !again.iter().any(|cmd| matches!(cmd, Cmd::SyncNow(_))),
            "a pending sync is not asked again"
        );
    }

    #[test]
    fn transitions_done_drops_prev_list_and_closing_overlays() {
        let (tabs, mut state) = setup(None);
        let overlay = |kind, closing| Overlay {
            kind,
            closing,
            state: Box::new(()),
        };
        state.ui.prev_list = Some((empty_list(), Direction::Right));
        state.ui.overlays = vec![
            overlay(OverlayKind::Popup, false),
            overlay(OverlayKind::Confirm, true),
        ];
        assert_eq!(run(&mut state, &tabs, Msg::TransitionsDone), vec![]);
        assert_eq!(state.ui.prev_list, None);
        assert_eq!(state.ui.overlays.len(), 1);
        assert_eq!(state.ui.overlays[0].kind, OverlayKind::Popup);
        assert_eq!(state.generation, 1);
        assert_eq!(run(&mut state, &tabs, Msg::TransitionsDone), vec![]);
        assert_eq!(state.generation, 1, "nothing left to drop");
    }

    fn at(secs: i64) -> DateTime<Utc> {
        test_time() + chrono::TimeDelta::seconds(secs)
    }

    /// A pull request with every group observed at `secs`.
    fn pr_at(number: u64, title: &str, secs: i64) -> PullRequest {
        let mut pr = testdata::pr("chaoshub", number, title);
        pr.head.observed_at = at(secs);
        pr.ci_state.observed_at = at(secs);
        pr.checks.observed_at = at(secs);
        pr
    }

    fn fake_interest() -> Cmd {
        Cmd::SetInterest(Interest {
            tab: "one",
            scope: None,
            wants_local: false,
            selected: None,
            visible: Vec::new(),
        })
    }

    fn synced(prs: Vec<PullRequest>, gone: Option<Vec<PrKey>>, started: i64) -> Msg {
        Msg::PrsSynced {
            scope: Scope::Mine,
            prs,
            gone,
            started_at: at(started),
        }
    }

    #[test]
    fn prs_synced_stores_the_prs_and_scans_each_repo_once() {
        let (tabs, mut state) = setup(None);
        let mut other = pr_at(3, "Other repo", 0);
        other.key.repo = testdata::repo_key("codeview");
        let prs = vec![pr_at(1, "One", 0), pr_at(2, "Two", 0), other];
        assert_eq!(
            run(&mut state, &tabs, synced(prs, None, 0)),
            vec![
                Cmd::ScanJj(testdata::repo_key("chaoshub")),
                Cmd::ScanJj(testdata::repo_key("codeview")),
                fake_interest(),
            ]
        );
        assert_eq!(state.data.prs.len(), 3);
        assert_eq!(
            state.data.scopes.get(&testdata::pr_key("chaoshub", 1)),
            Some(&Scope::Mine)
        );
        assert_eq!(state.data.synced.get(&Scope::Mine), Some(&at(0)));
        assert_eq!(state.data.revision, 1);
        assert_eq!(state.generation, 1);
        let settings = state.data.repos[&testdata::repo_key("chaoshub")]
            .settings
            .clone();
        assert_eq!(settings, Some(pr_at(1, "One", 0).head.value.repo_settings));
    }

    #[test]
    fn older_groups_are_dropped_one_by_one() {
        let (tabs, mut state) = setup(None);
        run(
            &mut state,
            &tabs,
            synced(vec![pr_at(1, "New", 10)], None, 10),
        );
        let mut older = pr_at(1, "Old", 5);
        older.ci_state = Observed {
            value: crate::domain::pr::CiState::Success,
            observed_at: at(20),
        };
        run(&mut state, &tabs, synced(vec![older], None, 5));
        let pr = &state.data.prs[&testdata::pr_key("chaoshub", 1)];
        assert_eq!(pr.head.value.title, "New", "the older head is dropped");
        assert_eq!(pr.head.observed_at, at(10));
        assert_eq!(
            pr.ci_state.value,
            crate::domain::pr::CiState::Success,
            "the newer CI state applies"
        );
        assert_eq!(state.data.revision, 2);
    }

    #[test]
    fn the_same_data_again_does_not_move_the_revision() {
        let (tabs, mut state) = setup(None);
        run(&mut state, &tabs, synced(vec![pr_at(1, "One", 0)], None, 0));
        run(&mut state, &tabs, synced(vec![pr_at(1, "One", 5)], None, 5));
        assert_eq!(state.data.revision, 1);
        assert_eq!(
            state.data.prs[&testdata::pr_key("chaoshub", 1)]
                .head
                .observed_at,
            at(5)
        );
    }

    #[test]
    fn gone_drops_only_candidates_seen_before_the_sync_started() {
        let (tabs, mut state) = setup(None);
        let prs = vec![pr_at(1, "Old", 0), pr_at(2, "Refreshed", 30)];
        run(&mut state, &tabs, synced(prs, None, 0));
        let gone = vec![
            testdata::pr_key("chaoshub", 1),
            testdata::pr_key("chaoshub", 2),
            testdata::pr_key("chaoshub", 9),
        ];
        let cmds = run(&mut state, &tabs, synced(Vec::new(), Some(gone), 20));
        assert_eq!(cmds, vec![]);
        let keys: Vec<_> = state.data.prs.keys().map(|key| key.number).collect();
        assert_eq!(keys, [2], "#2 was refreshed after the sync started");
        assert_eq!(state.data.scopes.len(), 1);
    }

    #[test]
    fn gone_drops_a_candidate_observed_at_the_start_time() {
        let (tabs, mut state) = setup(None);
        run(
            &mut state,
            &tabs,
            synced(vec![pr_at(1, "One", 20)], None, 0),
        );
        let gone = Some(vec![testdata::pr_key("chaoshub", 1)]);
        run(&mut state, &tabs, synced(Vec::new(), gone, 20));
        assert_eq!(
            state.data.prs.len(),
            0,
            "observed_at == started_at counts as not seen"
        );
    }

    #[test]
    fn pr_refreshed_merges_and_picks_a_scope() {
        let (tabs, mut state) = setup(None);
        state.data.session.login = Some("chaosteil".into());
        let mut colleague = pr_at(2, "Theirs", 0);
        colleague.head.value.author = "alice".into();
        let cmds = run(&mut state, &tabs, Msg::PrRefreshed(Box::new(colleague)));
        assert_eq!(cmds, vec![fake_interest()]);
        let cmds = run(
            &mut state,
            &tabs,
            Msg::PrRefreshed(Box::new(pr_at(1, "Mine", 0))),
        );
        assert_eq!(cmds, vec![], "the interest did not change");
        let scope = |n| state.data.scopes[&testdata::pr_key("chaoshub", n)];
        assert_eq!(scope(1), Scope::Mine);
        assert_eq!(scope(2), Scope::Team);
        let generation = state.generation;
        let older = pr_at(1, "Older", -10);
        assert_eq!(
            run(&mut state, &tabs, Msg::PrRefreshed(Box::new(older))),
            vec![]
        );
        assert_eq!(state.generation, generation, "older data is not visible");
    }

    fn stacked(number: u64, position: u32) -> PullRequest {
        let mut pr = pr_at(number, "Stacked", 0);
        pr.head.value.stack = Some(crate::domain::stack::StackRef {
            id: "S_1".into(),
            number: 7,
            size: 3,
            position,
            base_ref: "main".into(),
        });
        pr
    }

    #[test]
    fn follow_up_refreshes_the_stack_group_and_scans_the_repo() {
        let (tabs, mut state) = setup(None);
        let prs = vec![
            stacked(13, 3),
            stacked(11, 1),
            stacked(12, 2),
            pr_at(5, "Single", 0),
        ];
        run(&mut state, &tabs, synced(prs.clone(), None, 0));
        let done = |key| Msg::ActionEvent {
            id: "approve",
            key,
            event: ActionEvent::Done(Ok("approved".into())),
        };
        let repo = testdata::repo_key("chaoshub");
        let refs = |numbers: &[u64]| {
            numbers
                .iter()
                .map(|&n| pr_ref(&state.data.prs[&testdata::pr_key("chaoshub", n)]))
                .collect::<Vec<_>>()
        };
        let stack_refs = refs(&[11, 12, 13]);
        let single_refs = refs(&[5]);
        let key = |n| TargetKey::Pr(testdata::pr_key("chaoshub", n));
        assert_eq!(
            run(&mut state, &tabs, done(key(12))),
            vec![Cmd::ScanJj(repo.clone()), Cmd::RefreshPr(stack_refs)]
        );
        assert_eq!(
            run(
                &mut state,
                &tabs,
                Msg::PaneEnded {
                    id: "review",
                    key: key(5)
                }
            ),
            vec![Cmd::ScanJj(repo.clone()), Cmd::RefreshPr(single_refs)]
        );
        assert_eq!(
            run(&mut state, &tabs, done(key(99))),
            vec![Cmd::ScanJj(repo.clone())],
            "an unknown pull request has no refs"
        );
        let local = TargetKey::Local {
            repo: repo.clone(),
            change_id: "kxyzkxyz".into(),
        };
        assert_eq!(run(&mut state, &tabs, done(local)), vec![Cmd::ScanJj(repo)]);
        let progress = Msg::ActionEvent {
            id: "approve",
            key: key(5),
            event: ActionEvent::Progress("working".into()),
        };
        assert_eq!(run(&mut state, &tabs, progress), vec![]);
    }

    /// The real tabs with the mine fixtures, on a screen with a list of
    /// `height` rows.
    fn mine_state(height: u16) -> AppState {
        let mut state = AppState::new(Size {
            width: 100,
            height: height + 2,
        });
        state.load(testdata::mine_snapshot(), Vec::new());
        state
    }

    fn key(state: &mut AppState, c: char) -> Vec<Cmd> {
        update(
            state,
            &Keymap::default(),
            press(KeyCode::Char(c), KeyModifiers::NONE),
        )
    }

    fn selected_number(state: &mut AppState) -> Option<u64> {
        let model = frame::active_list(state, &TABS);
        match &model.rows[model.cursor].item {
            crate::app::models::RowItem::Pr(pr) => Some(pr.key.number),
            _ => None,
        }
    }

    #[test]
    fn j_and_k_move_over_the_rows_and_skip_headers() {
        // Rows: #99, header, #103, #102, #101, #7.
        let mut state = mine_state(10);
        assert_eq!(selected_number(&mut state), Some(99));
        let cmds = key(&mut state, 'j');
        assert_eq!(selected_number(&mut state), Some(103));
        let selected = state.data.prs[&testdata::pr_key("chaoshub", 103)].clone();
        assert!(matches!(
            cmds.as_slice(),
            [Cmd::SetInterest(interest)] if interest.selected == Some(pr_ref(&selected))
                && interest.visible.len() == 5
                && interest.tab == "mine"
        ));
        key(&mut state, 'G');
        assert_eq!(selected_number(&mut state), Some(7));
        key(&mut state, 'k');
        assert_eq!(selected_number(&mut state), Some(101));
        key(&mut state, 'g');
        assert_eq!(selected_number(&mut state), Some(99));
        let generation = state.generation;
        assert_eq!(key(&mut state, 'k'), vec![], "no move at the top");
        assert_eq!(state.generation, generation);
        assert_eq!(state.ui.ui_events, vec![], "no scroll in a tall list");
    }

    #[test]
    fn a_move_out_of_the_window_scrolls_with_an_event() {
        let mut state = mine_state(4);
        key(&mut state, 'j');
        key(&mut state, 'j');
        assert_eq!(state.ui.ui_events, vec![], "#103 and #102 are visible");
        key(&mut state, 'j');
        assert_eq!(
            state.ui.ui_events,
            vec![UiEvent::ScrollTo {
                tab: "mine",
                from: 0,
                to: 1,
            }]
        );
        assert_eq!(frame::active_list(&mut state, &TABS).scroll, 1);
        let ctrl_u = press(KeyCode::Char('u'), KeyModifiers::CONTROL);
        update(&mut state, &Keymap::default(), ctrl_u);
        assert_eq!(selected_number(&mut state), Some(103));
    }

    #[test]
    fn the_selection_survives_a_sync() {
        let mut state = mine_state(10);
        key(&mut state, 'j');
        key(&mut state, 'j');
        assert_eq!(selected_number(&mut state), Some(102));
        let mut newer = testdata::pr("chaoshub", 200, "A new one on top");
        newer.head.value.updated_at += TimeDelta::days(1);
        newer.head.observed_at += TimeDelta::seconds(1);
        let mut moved = state.data.prs[&testdata::pr_key("chaoshub", 102)].clone();
        moved.head.value.head_oid = "pushed".into();
        moved.head.observed_at += TimeDelta::seconds(1);
        let msg = Msg::PrsSynced {
            scope: Scope::Mine,
            prs: vec![newer, moved.clone()],
            gone: None,
            started_at: at(1),
        };
        let cmds = update(&mut state, &Keymap::default(), msg);
        assert_eq!(selected_number(&mut state), Some(102));
        assert_eq!(frame::active_list(&mut state, &TABS).cursor, 4);
        let interest = cmds.iter().find_map(|cmd| match cmd {
            Cmd::SetInterest(interest) => Some(interest.clone()),
            _ => None,
        });
        assert_eq!(
            interest.and_then(|interest| interest.selected),
            Some(pr_ref(&moved)),
            "the interest carries the new head"
        );
    }

    #[test]
    fn rate_info_is_stored_without_a_redraw() {
        let (tabs, mut state) = setup(None);
        let info = RateInfo {
            remaining: 4000,
            reset_at: test_time(),
            last_cost: 1,
        };
        assert_eq!(run(&mut state, &tabs, Msg::Rate(info.clone())), vec![]);
        assert_eq!(state.data.sync.rate, Some(info));
        assert_eq!(state.generation, 0);
    }
}
