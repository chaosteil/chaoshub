//! The app: state, `update`, the command runtime, tabs, panes, models and
//! layout. No ratatui.

mod age;
pub(crate) mod exec;
mod frame;
pub(crate) mod keymap;
pub(crate) mod layout;
pub(crate) mod models;
mod nav;
mod panes;
mod state;
mod tabs;
mod term;
#[cfg(test)]
pub(crate) mod testdata;
mod update;

use chrono::{DateTime, Utc};

use crate::app::keymap::Keymap;
use crate::app::models::FrameModel;
use crate::app::state::AppState;
use crate::bus::{Cmd, Msg, Size, UiEvent};
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

    /// The redraw signal.
    pub(crate) fn generation(&self) -> u64 {
        self.state.generation
    }

    /// The terminal size.
    pub(crate) fn size(&self) -> Size {
        self.state.data.size
    }

    /// Takes the UI events for the transitions.
    pub(crate) fn take_ui_events(&mut self) -> Vec<UiEvent> {
        std::mem::take(&mut self.state.ui.ui_events)
    }
}
