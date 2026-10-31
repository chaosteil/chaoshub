//! The app state: data and UI state.
//!
//! The split lets `update` borrow `&state.data` and `&mut state.ui` at the
//! same time, for example to give a tab read access to the data while it
//! changes its own state.
#![allow(
    dead_code,
    reason = "update, the tabs and the panes read the fields in later phases"
)]

use std::any::Any;
use std::collections::BTreeMap;

use crate::app::frame::ModelCache;
use crate::app::models::{Direction, ListModel};
use crate::app::tabs::{TABS, Tab, TabState};
use crate::bus::{OverlayKind, Session, Size, UiEvent};
use crate::domain::detail::PrDetail;
use crate::domain::keys::{PrKey, RepoKey};
use crate::domain::local::LocalState;
use crate::domain::observed::Observed;
use crate::domain::pr::PullRequest;
use crate::domain::repo::Repo;

/// The whole state of the app.
#[derive(Debug)]
pub(crate) struct AppState {
    pub data: AppData,
    pub ui: UiState,
    /// The redraw signal. `update` bumps it on every visible change, and
    /// `run` draws when it moved since the last draw.
    pub generation: u64,
    /// The models of the last frame.
    pub cache: ModelCache,
}

/// The data that the workers deliver.
#[derive(Debug)]
pub(crate) struct AppData {
    /// The terminal size.
    pub size: Size,
    pub session: Session,
    pub prs: BTreeMap<PrKey, PullRequest>,
    /// The jj scan of each repo.
    pub local: BTreeMap<RepoKey, Observed<LocalState>>,
    pub details: BTreeMap<PrKey, Observed<PrDetail>>,
    pub repos: BTreeMap<RepoKey, Repo>,
}

/// The state of the UI.
#[derive(Debug)]
pub(crate) struct UiState {
    /// The index of the active tab in the tab registry.
    pub active: usize,
    /// One state per tab, in registry order.
    pub tabs: Vec<TabState>,
    /// The overlay stack, bottom first.
    pub overlays: Vec<Overlay>,
    /// The last status message.
    pub status: Option<String>,
    /// The events for the transitions since the last draw.
    pub ui_events: Vec<UiEvent>,
    /// The list of the previous tab and the slide direction, until the tab
    /// slide is done.
    pub prev_list: Option<(ListModel, Direction)>,
}

/// One overlay of the overlay stack.
#[derive(Debug)]
pub(crate) struct Overlay {
    pub kind: OverlayKind,
    /// True while the overlay slides out. `Msg::TransitionsDone` removes
    /// it.
    pub closing: bool,
    /// The own state of the overlay.
    pub state: Box<dyn Any + Send + Sync>,
}

impl AppState {
    /// Creates an empty state for a terminal size, with one tab state
    /// per tab of `TABS`.
    pub(crate) fn new(size: Size) -> Self {
        Self::with_tabs(size, &TABS)
    }

    /// Creates an empty state with one tab state per tab of `tabs`.
    pub(crate) fn with_tabs(size: Size, tabs: &[Box<dyn Tab>]) -> Self {
        Self {
            data: AppData {
                size,
                session: Session {
                    login: None,
                    repos: Vec::new(),
                },
                prs: BTreeMap::new(),
                local: BTreeMap::new(),
                details: BTreeMap::new(),
                repos: BTreeMap::new(),
            },
            ui: UiState {
                active: 0,
                tabs: tabs.iter().map(|tab| tab.init_state()).collect(),
                overlays: Vec::new(),
                status: None,
                ui_events: Vec::new(),
                prev_list: None,
            },
            generation: 0,
            cache: ModelCache::default(),
        }
    }

    /// Marks a visible change.
    pub(crate) fn bump(&mut self) {
        self.generation += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tabs::fake::FakeTab;

    #[test]
    fn new_state_is_empty_and_bump_moves_the_generation() {
        let size = Size {
            width: 100,
            height: 30,
        };
        let mut state = AppState::new(size);
        assert_eq!(state.data.size, size);
        assert!(state.data.prs.is_empty());
        assert!(state.ui.overlays.is_empty());
        assert_eq!(state.generation, 0);
        state.bump();
        assert_eq!(state.generation, 1);
    }

    #[test]
    fn with_tabs_makes_one_state_per_tab() {
        let tabs: Vec<Box<dyn Tab>> =
            vec![Box::new(FakeTab::new("a")), Box::new(FakeTab::new("b"))];
        let state = AppState::with_tabs(
            Size {
                width: 100,
                height: 30,
            },
            &tabs,
        );
        assert_eq!(state.ui.tabs.len(), 2);
        assert_eq!(state.ui.active, 0);
    }
}
