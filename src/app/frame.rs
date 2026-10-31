//! Builds the frame model from the app state.
//!
//! The frame model is rebuilt only when the generation moved. The list
//! model of each tab is cached with the generation and the size it was
//! built for, so animation frames and idle ticks never rebuild it.

use crate::app::layout::layout;
use crate::app::models::{Cell, CellStyle, FrameModel, ListModel, StatusModel, TabBarModel};
use crate::app::state::AppState;
use crate::app::tabs::{KeyCx, TABS, Tab};
use crate::app::term::term_status;
use crate::bus::{Progress, Size, TermState, TermStatus};

/// The models of the last frame.
#[derive(Debug)]
pub(crate) struct ModelCache {
    /// The list model of each tab, by registry index.
    lists: Vec<Option<CachedList>>,
    /// The generation that `frame` was built for. `None` before the first
    /// build.
    frame_generation: Option<u64>,
    frame: FrameModel,
}

impl Default for ModelCache {
    fn default() -> Self {
        Self {
            lists: Vec::new(),
            frame_generation: None,
            frame: empty_frame(),
        }
    }
}

/// A list model and the generation and the size that it was built for.
#[derive(Debug)]
struct CachedList {
    generation: u64,
    size: Size,
    model: ListModel,
}

/// The frame model for the current generation, from the tabs of `TABS`.
pub(crate) fn frame_model(state: &mut AppState) -> &FrameModel {
    frame_model_with(state, &TABS)
}

/// The frame model for the current generation, from the tabs of `tabs`.
pub(crate) fn frame_model_with<'s>(
    state: &'s mut AppState,
    tabs: &[Box<dyn Tab>],
) -> &'s FrameModel {
    if state.cache.frame_generation != Some(state.generation) {
        let areas = layout(state.data.size);
        let size = Size {
            width: areas.list.width,
            height: areas.list.height,
        };
        let list = list_model(state, tabs, state.ui.active, size);
        let frame = FrameModel {
            tab_bar: TabBarModel {
                titles: tabs.iter().map(|tab| tab.title()).collect(),
                active: state.ui.active,
            },
            list,
            prev_list: state.ui.prev_list.clone(),
            status: status_model(state, tabs),
            // The overlay models come with the overlay kinds of phase D.
            overlays: Vec::new(),
            term: term_status(state),
        };
        state.cache.frame = frame;
        state.cache.frame_generation = Some(state.generation);
    }
    &state.cache.frame
}

/// The list model of the tab at `index` for the list area `size`, from the
/// cache when it was built for the same generation and size.
fn list_model(state: &mut AppState, tabs: &[Box<dyn Tab>], index: usize, size: Size) -> ListModel {
    let generation = state.generation;
    if let Some(Some(cached)) = state.cache.lists.get(index)
        && cached.generation == generation
        && cached.size == size
    {
        return cached.model.clone();
    }
    let (Some(tab), Some(ts)) = (tabs.get(index), state.ui.tabs.get(index)) else {
        return empty_list();
    };
    let areas = layout(state.data.size);
    let cx = KeyCx {
        data: &state.data,
        target: None,
        areas: &areas,
    };
    let model = tab.model(&cx, ts, size);
    if state.cache.lists.len() <= index {
        state.cache.lists.resize_with(index + 1, || None);
    }
    state.cache.lists[index] = Some(CachedList {
        generation,
        size,
        model: model.clone(),
    });
    model
}

/// The status bar: the active tab on the left, the last message in the
/// center.
pub(crate) fn status_model(state: &AppState, tabs: &[Box<dyn Tab>]) -> StatusModel {
    StatusModel {
        left: tabs
            .get(state.ui.active)
            .map(|tab| Cell {
                text: format!("[{}]", tab.title().to_uppercase()),
                style: CellStyle::Primary,
            })
            .into_iter()
            .collect(),
        center: state.ui.status.as_ref().map(|msg| Cell {
            text: msg.clone(),
            style: CellStyle::Normal,
        }),
        right: Vec::new(),
    }
}

/// A list with no columns and no rows.
fn empty_list() -> ListModel {
    ListModel {
        columns: Vec::new(),
        rows: Vec::new(),
        cursor: 0,
        scroll: 0,
    }
}

/// A frame with no content.
fn empty_frame() -> FrameModel {
    FrameModel {
        tab_bar: TabBarModel {
            titles: Vec::new(),
            active: 0,
        },
        list: empty_list(),
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::app::models::Direction;
    use crate::app::tabs::fake::FakeTab;

    const SIZE: Size = Size {
        width: 100,
        height: 30,
    };

    /// Two fake tabs, a state for them, and the build counter of each tab.
    fn fake_tabs() -> (Vec<Box<dyn Tab>>, AppState, [Arc<AtomicUsize>; 2]) {
        let one = FakeTab::new("one");
        let two = FakeTab::new("two");
        let counters = [one.counter(), two.counter()];
        let tabs: Vec<Box<dyn Tab>> = vec![Box::new(one), Box::new(two)];
        let state = AppState::with_tabs(SIZE, &tabs);
        (tabs, state, counters)
    }

    fn builds(counter: &AtomicUsize) -> usize {
        counter.load(Ordering::SeqCst)
    }

    #[test]
    fn frame_is_built_once_per_generation() {
        let (tabs, mut state, [one, two]) = fake_tabs();
        frame_model_with(&mut state, &tabs);
        frame_model_with(&mut state, &tabs);
        assert_eq!(builds(&one), 1);
        assert_eq!(builds(&two), 0);
        state.bump();
        frame_model_with(&mut state, &tabs);
        assert_eq!(builds(&one), 2);
    }

    #[test]
    fn list_is_cached_by_generation_and_size() {
        let (tabs, mut state, [one, _]) = fake_tabs();
        let small = Size {
            width: 40,
            height: 10,
        };
        list_model(&mut state, &tabs, 0, SIZE);
        list_model(&mut state, &tabs, 0, SIZE);
        assert_eq!(builds(&one), 1);
        list_model(&mut state, &tabs, 0, small);
        assert_eq!(builds(&one), 2);
        list_model(&mut state, &tabs, 0, small);
        assert_eq!(builds(&one), 2);
        state.bump();
        list_model(&mut state, &tabs, 0, small);
        assert_eq!(builds(&one), 3);
    }

    #[test]
    fn list_of_a_missing_tab_is_empty() {
        let (tabs, mut state, _) = fake_tabs();
        assert_eq!(list_model(&mut state, &tabs, 5, SIZE), empty_list());
    }

    #[test]
    fn frame_has_the_tab_bar_status_and_prev_list() {
        let (tabs, mut state, [one, two]) = fake_tabs();
        state.ui.active = 1;
        state.ui.status = Some("synced".into());
        state.ui.prev_list = Some((empty_list(), Direction::Left));
        let frame = frame_model_with(&mut state, &tabs).clone();
        assert_eq!(builds(&one), 0);
        assert_eq!(builds(&two), 1);
        assert_eq!(
            frame.tab_bar,
            TabBarModel {
                titles: vec!["one", "two"],
                active: 1,
            }
        );
        assert_eq!(
            frame.status,
            StatusModel {
                left: vec![Cell {
                    text: "[TWO]".into(),
                    style: CellStyle::Primary,
                }],
                center: Some(Cell {
                    text: "synced".into(),
                    style: CellStyle::Normal,
                }),
                right: Vec::new(),
            }
        );
        assert_eq!(frame.prev_list, Some((empty_list(), Direction::Left)));
        assert_eq!(frame.term, term_status(&state));
    }

    #[test]
    fn empty_registry_gives_an_empty_frame() {
        let tabs: Vec<Box<dyn Tab>> = Vec::new();
        let mut state = AppState::with_tabs(SIZE, &tabs);
        assert_eq!(frame_model_with(&mut state, &tabs), &empty_frame());
    }
}
