//! The app: state, `update`, the command runtime, tabs, panes, models and
//! layout. No ratatui.

pub(crate) mod exec;
mod frame;
mod keymap;
pub(crate) mod layout;
pub(crate) mod models;
mod panes;
mod state;
mod tabs;
mod term;
mod update;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::models::{FrameModel, ListModel, StatusModel, TabBarModel};
use crate::app::state::AppState;
use crate::bus::{Cmd, Msg, Progress, Size, TermState, TermStatus, UiEvent};

/// The app: the state and the message handler. `run` drives it.
#[derive(Debug)]
pub(crate) struct App {
    state: AppState,
    frame: FrameModel,
}

impl App {
    /// Creates the app for a terminal size.
    pub(crate) fn new(size: Size) -> Self {
        Self {
            state: AppState::new(size),
            frame: empty_frame(),
        }
    }

    /// Applies one message and returns the commands to execute.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "the data messages of later phases move their payload into the state"
    )]
    pub(crate) fn update(&mut self, msg: Msg) -> Vec<Cmd> {
        match msg {
            Msg::Key(event) if is_quit(&event) => vec![Cmd::Quit],
            Msg::Signal => vec![Cmd::Quit],
            Msg::Resize(size) => {
                self.state.data.size = size;
                self.state.bump();
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// The model of the next frame.
    pub(crate) fn frame(&mut self) -> &FrameModel {
        &self.frame
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

/// True for `q` and `Ctrl-c`.
fn is_quit(event: &KeyEvent) -> bool {
    event.kind != KeyEventKind::Release
        && matches!(
            (event.code, event.modifiers),
            (KeyCode::Char('q'), KeyModifiers::NONE) | (KeyCode::Char('c'), KeyModifiers::CONTROL)
        )
}

/// A frame with no content.
fn empty_frame() -> FrameModel {
    FrameModel {
        tab_bar: TabBarModel {
            titles: Vec::new(),
            active: 0,
        },
        list: ListModel {
            columns: Vec::new(),
            rows: Vec::new(),
            cursor: 0,
            scroll: 0,
        },
        prev_list: None,
        status: StatusModel {
            left: Vec::new(),
            center: None,
            right: Vec::new(),
        },
        overlays: Vec::new(),
        term: TermStatus {
            state: TermState::Idle,
            msg: None,
            progress: Progress::Off,
        },
    }
}
