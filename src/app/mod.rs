//! The app: state, `update`, the command runtime, tabs, panes, models and
//! layout. No ratatui.

mod age;
pub(crate) mod exec;
mod filter;
mod frame;
pub(crate) mod keymap;
pub(crate) mod layout;
mod merge;
pub(crate) mod models;
mod nav;
mod panes;
mod state;
mod tabs;
mod target;
mod term;
#[cfg(test)]
pub(crate) mod testdata;
mod update;

use chrono::{DateTime, Utc};

use crate::app::keymap::Keymap;
use crate::app::models::FrameModel;
use crate::app::state::AppState;
use crate::bus::{Cmd, Interest, Msg, Size, UiEvent};
use crate::domain::repo::Repo;
use crate::store::Snapshot;

/// The app: the state and the message handler. `run` drives it.
#[derive(Debug)]
pub(crate) struct App {
    state: AppState,
    keymap: Keymap,
}

impl App {
    /// Creates the app for a terminal size and a key map, with the
    /// startup snapshot of the cache and the repos of the session. `now`
    /// is the start time; the ages of the first frame count from it.
    pub(crate) fn new(
        size: Size,
        keymap: Keymap,
        snapshot: Snapshot,
        repos: Vec<Repo>,
        now: DateTime<Utc>,
    ) -> Self {
        let mut state = AppState::new(size);
        state.data.now = now;
        state.load(snapshot, repos);
        update::start(&mut state);
        Self { state, keymap }
    }

    /// Applies one message and returns the commands to execute.
    pub(crate) fn update(&mut self, msg: Msg) -> Vec<Cmd> {
        update::update(&mut self.state, &self.keymap, msg)
    }

    /// The model of the next frame, rebuilt only when the generation
    /// moved.
    pub(crate) fn frame(&mut self) -> &FrameModel {
        frame::frame_model(&mut self.state)
    }

    /// The interest that `update` computed last, for the first value of
    /// the interest watch.
    pub(crate) fn interest(&self) -> Option<&Interest> {
        self.state.ui.interest.as_ref()
    }

    /// The redraw signal.
    pub(crate) fn generation(&self) -> u64 {
        self.state.generation
    }

    /// The terminal size.
    pub(crate) fn size(&self) -> Size {
        self.state.data.size
    }

    /// True when the frame still holds what a transition shows: the list
    /// of the previous tab or an overlay that slides out. Without a
    /// running transition, `run` then sends `Msg::TransitionsDone`.
    pub(crate) fn transition_leftovers(&self) -> bool {
        self.state.ui.prev_list.is_some() || self.state.ui.overlays.iter().any(|o| o.closing)
    }

    /// Takes the UI events for the transitions.
    pub(crate) fn take_ui_events(&mut self) -> Vec<UiEvent> {
        std::mem::take(&mut self.state.ui.ui_events)
    }
}
