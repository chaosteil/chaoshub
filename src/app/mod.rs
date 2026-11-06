//! The app: state, `update`, the command runtime, tabs, panes, models and
//! layout. No ratatui.

mod age;
pub(crate) mod exec;
mod filter;
mod frame;
mod help;
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

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

pub(crate) use crate::app::frame::SPINNER_FRAME;
use crate::app::keymap::Keymap;
use crate::app::layout::Areas;
use crate::app::models::FrameModel;
use crate::app::state::AppState;
pub(crate) use crate::app::state::DetailPane;
use crate::bus::{Cmd, Interest, Msg, Size, UiEvent};
use crate::domain::keys::PrKey;
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
    /// `key_hints` is the `[theme] key_hints` switch.
    pub(crate) fn new(
        size: Size,
        keymap: Keymap,
        key_hints: bool,
        snapshot: Snapshot,
        repos: Vec<Repo>,
        now: DateTime<Utc>,
    ) -> Self {
        let mut state = AppState::new(size);
        state.cache.set_key_hints(keymap.clone(), key_hints);
        state.data.now = now;
        state.load(snapshot, repos);
        update::start(&mut state);
        Self { state, keymap }
    }

    /// Sets the `[actions.<id>]` tables of the configuration. Each
    /// selection carries the table of its action.
    pub(crate) fn with_action_config(mut self, tables: BTreeMap<String, toml::Table>) -> Self {
        self.state.data.action_config = tables;
        self
    }

    /// True when the list shows the header row.
    #[cfg(test)]
    pub(crate) fn column_headers(&self) -> bool {
        self.state.ui.column_headers
    }

    /// Sets the `[theme] column_headers` switch. A value in the cache wins
    /// over it.
    pub(crate) fn with_column_headers(mut self, on: bool) -> Self {
        let ui = &mut self.state.ui;
        ui.column_headers = ui.saved_column_headers.unwrap_or(on);
        self.state.bump();
        self
    }

    /// Sets the command of the detail pane. With `Some`, `enter` opens the
    /// pull request in a pane. With `None`, it opens the sidebar.
    pub(crate) fn with_detail_pane(mut self, detail: Option<DetailPane>) -> Self {
        self.state.data.detail = detail;
        self
    }

    /// Pins the pull request `key`: the pinned mode of `chaoshub pr`. The
    /// popup of `key` is in place from the first frame, without a slide,
    /// and the interest is the one of the pinned pull request.
    pub(crate) fn pinned(mut self, key: PrKey) -> Self {
        update::pin(&mut self.state, key);
        self
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

    /// True while a requested sync or a prefetch batch runs: the status
    /// bar shows a spinner, and `run` ticks every `SPINNER_FRAME`.
    pub(crate) fn busy(&self) -> bool {
        self.state.data.busy()
    }

    /// The redraw signal.
    pub(crate) fn generation(&self) -> u64 {
        self.state.generation
    }

    /// The terminal size.
    #[cfg(test)]
    pub(crate) fn size(&self) -> Size {
        self.state.data.size
    }

    /// The areas of the screen, with the key hint bar when the key hints
    /// are on.
    pub(crate) fn areas(&self) -> Areas {
        frame::areas(&self.state)
    }

    /// True when the frame still holds what a transition shows: an
    /// overlay that slides out. Without a running transition, `run` then
    /// sends `Msg::TransitionsDone`.
    pub(crate) fn transition_leftovers(&self) -> bool {
        self.state.ui.overlays.iter().any(|o| o.closing)
    }

    /// Takes the UI events for the transitions.
    pub(crate) fn take_ui_events(&mut self) -> Vec<UiEvent> {
        std::mem::take(&mut self.state.ui.ui_events)
    }
}
