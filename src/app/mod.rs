//! The app: state, `update`, the command runtime, tabs, panes, models and
//! layout. No ratatui.

pub(crate) mod exec;
mod frame;
pub(crate) mod keymap;
pub(crate) mod layout;
pub(crate) mod models;
mod panes;
mod state;
mod tabs;
mod term;
mod update;

use crate::app::keymap::Keymap;
use crate::app::models::FrameModel;
use crate::app::state::AppState;
use crate::bus::{Cmd, Msg, Size, UiEvent};

/// The app: the state and the message handler. `run` drives it.
#[derive(Debug)]
pub(crate) struct App {
    state: AppState,
    keymap: Keymap,
}

impl App {
    /// Creates the app for a terminal size and a key map.
    pub(crate) fn new(size: Size, keymap: Keymap) -> Self {
        Self {
            state: AppState::new(size),
            keymap,
        }
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
