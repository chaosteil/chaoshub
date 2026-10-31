//! The `update` function: applies a `Msg` and returns commands.
//!
//! There is one function per message family. Commands are plain data, so
//! the tests compare them with `assert_eq!`.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};

use crate::app::frame;
use crate::app::keymap::{self, Keymap};
use crate::app::layout::layout;
use crate::app::state::{AppData, AppState};
use crate::app::tabs::{KeyCx, TABS, Tab};
use crate::bus::{ActionEvent, Cmd, KeyEvent, KeyEventKind, Msg, PrRef, Scope, Size, TargetKey};
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
        Msg::Tick => tick(state, tabs),
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
    let chord = keymap::chord(event);
    if keymap.matches("quit", chord) {
        return quit();
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

/// The fields of a pull request that the workers need.
pub(crate) fn pr_ref(pr: &PullRequest) -> PrRef {
    PrRef {
        key: pr.key.clone(),
        node_id: pr.node_id.clone(),
        head_oid: pr.head.value.head_oid.clone(),
        base_ref: pr.head.value.base_ref.clone(),
    }
}

/// A new terminal size.
fn resize(state: &mut AppState, size: Size) -> Vec<Cmd> {
    state.data.size = size;
    state.bump();
    Vec::new()
}

/// The deadline of the event loop. The generation moves only when a
/// rendered string changed since the last frame.
fn tick(state: &mut AppState, tabs: &[Box<dyn Tab>]) -> Vec<Cmd> {
    let status = frame::status_model(state, tabs);
    if frame::built_status(state).is_some_and(|built| *built != status) {
        state.bump();
    }
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
    use crate::bus::{Interest, KeyCode, KeyModifiers, OverlayKind, RateInfo, UiEvent};
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
        let j = press(KeyCode::Char('j'), KeyModifiers::NONE);
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
        let j = press(KeyCode::Char('j'), KeyModifiers::NONE);
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
        assert_eq!(run(&mut state, &tabs, Msg::Tick), vec![]);
        assert_eq!(state.generation, 0, "no frame was built yet");
        frame::frame_model_with(&mut state, &tabs);
        assert_eq!(run(&mut state, &tabs, Msg::Tick), vec![]);
        assert_eq!(state.generation, 0);
        state.ui.status = Some("synced 1m ago".into());
        assert_eq!(run(&mut state, &tabs, Msg::Tick), vec![]);
        assert_eq!(state.generation, 1);
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

    #[test]
    fn other_messages_do_nothing_yet() {
        let (tabs, mut state) = setup(None);
        let rate = Msg::Rate(RateInfo {
            remaining: 4000,
            reset_at: test_time(),
            last_cost: 1,
        });
        assert_eq!(run(&mut state, &tabs, rate), vec![]);
        assert_eq!(state.generation, 0);
    }
}
