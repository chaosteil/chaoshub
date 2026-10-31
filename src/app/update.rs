//! The `update` function: applies a `Msg` and returns commands.
//!
//! There is one function per message family. Commands are plain data, so
//! the tests compare them with `assert_eq!`.

use crate::app::frame;
use crate::app::keymap::{self, Keymap};
use crate::app::layout::layout;
use crate::app::state::AppState;
use crate::app::tabs::{KeyCx, TABS, Tab};
use crate::bus::{Cmd, KeyEvent, KeyEventKind, Msg, Size};

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
    let interest = tab.interest(&cx, ts);
    if handled.changed {
        state.bump();
    }
    state.ui.ui_events.extend(handled.events);
    let mut cmds = handled.cmds;
    if state.ui.interest.as_ref() != Some(&interest) {
        state.ui.interest = Some(interest.clone());
        cmds.push(Cmd::SetInterest(interest));
    }
    cmds
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
    use crate::bus::{Interest, KeyCode, KeyModifiers, OverlayKind, RateInfo, Scope, UiEvent};
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
