//! Builds the frame model from the app state.
//!
//! The frame model is rebuilt only when the generation moved. The list
//! model of each tab is cached with the generation and the size it was
//! built for, so animation frames and idle ticks never rebuild it.

use crate::app::age::age;
use crate::app::keymap;
use crate::app::layout::layout;
use crate::app::models::{
    Cell, CellStyle, FilterModel, FrameModel, ListModel, OverlayContent, OverlayModel, PaneModel,
    PaneTab, StatusModel, TabBarModel,
};
use crate::app::panes::conversation::is_stale;
use crate::app::panes::{PANES, Pane, applicable};
use crate::app::state::{AppData, AppState, OverlayState, PopupState};
use crate::app::tabs::{KeyCx, TABS, Tab};
use crate::app::target::pr_target;
use crate::app::term::term_status;
use crate::bus::{Progress, Size, Target, TermState, TermStatus};
use crate::domain::pr::PrState;

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
    frame_model_full(state, tabs, &PANES)
}

/// The frame model for the current generation, from the tabs of `tabs`
/// and the panes of `panes`.
pub(crate) fn frame_model_full<'s>(
    state: &'s mut AppState,
    tabs: &[Box<dyn Tab>],
    panes: &[Box<dyn Pane>],
) -> &'s FrameModel {
    if state.cache.frame_generation != Some(state.generation) {
        let list = active_list(state, tabs);
        let frame = FrameModel {
            tab_bar: TabBarModel {
                titles: tabs.iter().map(|tab| tab.title()).collect(),
                active: state.ui.active,
            },
            list,
            prev_list: state.ui.prev_list.clone(),
            status: status_model(state, tabs),
            overlays: overlay_models(state, panes),
            term: term_status(state),
            filter: filter_model(state),
        };
        state.cache.frame = frame;
        state.cache.frame_generation = Some(state.generation);
    }
    &state.cache.frame
}

/// The models of the overlay stack, bottom first.
fn overlay_models(state: &AppState, panes: &[Box<dyn Pane>]) -> Vec<OverlayModel> {
    state
        .ui
        .overlays
        .iter()
        .map(|overlay| OverlayModel {
            kind: overlay.kind,
            closing: overlay.closing,
            content: match &overlay.state {
                OverlayState::Popup(popup) => popup_content(&state.data, popup, panes),
                OverlayState::Authors(picker) => OverlayContent::Picker {
                    title: "author".into(),
                    items: picker.items(),
                    cursor: picker.cursor,
                },
                OverlayState::Confirm { spec, .. } => OverlayContent::Confirm(spec.clone()),
                OverlayState::Prompt { spec, text, .. } => OverlayContent::Prompt {
                    spec: spec.clone(),
                    text: text.clone(),
                },
                OverlayState::Help(help) => OverlayContent::Help(help.clone()),
                OverlayState::Output { title, lines, .. } => OverlayContent::Output {
                    title: title.clone(),
                    lines: lines.clone(),
                },
            },
        })
        .collect()
}

/// The content of the pull request popup: the header, the tabs of the
/// panes that apply, and the model of the active pane.
fn popup_content(data: &AppData, popup: &PopupState, panes: &[Box<dyn Pane>]) -> OverlayContent {
    let Some(target) = pr_target(data, &popup.key) else {
        return OverlayContent::Popup {
            title: format!("#{}", popup.key.number),
            subtitle: "this pull request is gone".into(),
            tabs: Vec::new(),
            active: 0,
            pane: PaneModel::Lines {
                lines: Vec::new(),
                wrap: false,
            },
            cursor: 0,
            scroll: 0,
        };
    };
    let Target::Pr { pr, .. } = &target else {
        unreachable!("pr_target builds a pull request target")
    };
    let head = &pr.head.value;
    let state = match head.state {
        PrState::Open if head.is_draft => "draft",
        PrState::Open => "open",
        PrState::Closed => "closed",
        PrState::Merged => "merged",
    };
    let old_detail = data
        .details
        .get(&pr.key)
        .is_some_and(|detail| is_stale(&detail.value, pr));
    let tag = if old_detail { " · stale" } else { "" };
    let order = applicable(panes, &target);
    let tabs = order
        .iter()
        .map(|&i| PaneTab {
            key: panes[i].key(),
            title: panes[i].title(),
        })
        .collect();
    let areas = layout(data.size);
    let cx = KeyCx {
        data,
        target: Some(&target),
        areas: &areas,
        keys: &keymap::DEFAULT,
    };
    let size = Size {
        width: areas.popup_body.width,
        height: areas.popup_body.height,
    };
    let (pane, cursor, scroll) = match (panes.get(popup.active), popup.panes.get(popup.active)) {
        (Some(pane), Some(ps)) if order.contains(&popup.active) => {
            (pane.model(&cx, ps, size), ps.cursor, ps.scroll)
        }
        _ => (
            PaneModel::Lines {
                lines: Vec::new(),
                wrap: false,
            },
            0,
            0,
        ),
    };
    OverlayContent::Popup {
        title: format!("#{} {}", pr.key.number, head.title),
        subtitle: format!(
            "{} · {} -> {} · {state}{tag}",
            head.author, head.head_ref, head.base_ref
        ),
        tabs,
        active: order.iter().position(|&i| i == popup.active).unwrap_or(0),
        pane,
        cursor,
        scroll,
    }
}

/// The size of the list area that the tab models fill: the list area
/// without the filter line when it shows.
pub(crate) fn list_size(state: &AppState) -> Size {
    let areas = layout(state.data.size);
    let filter = u16::from(filter_model(state).is_some());
    Size {
        width: areas.list.width,
        height: areas.list.height.saturating_sub(filter),
    }
}

/// The filter line of the active tab: while the user edits the filter, or
/// while it has text.
fn filter_model(state: &AppState) -> Option<FilterModel> {
    let ts = state.ui.tabs.get(state.ui.active)?;
    (state.ui.filter_edit || !ts.filter.is_empty()).then(|| FilterModel {
        text: ts.filter.clone(),
        editing: state.ui.filter_edit,
    })
}

/// The list model of the active tab for the current generation.
pub(crate) fn active_list(state: &mut AppState, tabs: &[Box<dyn Tab>]) -> ListModel {
    let size = list_size(state);
    list_model(state, tabs, state.ui.active, size)
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
        keys: &keymap::DEFAULT,
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

/// The status bar: the active tab, the repo count and the data age on
/// the left, the last message in the center, the help hint on the right.
pub(crate) fn status_model(state: &AppState, tabs: &[Box<dyn Tab>]) -> StatusModel {
    let mut left: Vec<Cell> = tabs
        .get(state.ui.active)
        .map(|tab| Cell {
            text: format!("[{}]", tab.title().to_uppercase()),
            style: CellStyle::Primary,
        })
        .into_iter()
        .collect();
    left.push(Cell {
        text: sync_text(state, tabs),
        style: CellStyle::Dim,
    });
    StatusModel {
        left,
        center: state.ui.status.as_ref().map(|msg| Cell {
            text: msg.text.clone(),
            style: msg.style,
        }),
        right: vec![Cell {
            text: "? help".into(),
            style: CellStyle::Dim,
        }],
    }
}

/// The repo count, the age of the data and the pull requests that the
/// prefetch batch has left, for example
/// ` 3 repos · synced 12s ago · prefetch 2`.
fn sync_text(state: &AppState, tabs: &[Box<dyn Tab>]) -> String {
    let data = &state.data;
    let repos = data.session.repos.len();
    let noun = if repos == 1 { "repo" } else { "repos" };
    let scope = tabs.get(state.ui.active).and_then(|tab| tab.scope());
    let text = match (
        scope.and_then(|scope| data.synced.get(&scope)),
        data.cache_at,
    ) {
        (Some(&at), _) => format!(" {repos} {noun} · synced {} ago", age(at, data.now)),
        (None, Some(at)) => format!(" {repos} {noun} · cache from {} ago", age(at, data.now)),
        (None, None) => format!(" {repos} {noun}"),
    };
    match data.prefetch {
        Some(batch) => format!("{text} · prefetch {}", batch.total - batch.done),
        None => text,
    }
}

/// The status bar and the terminal status of the last built frame, if
/// any.
pub(crate) fn built_status(state: &AppState) -> Option<(&StatusModel, &TermStatus)> {
    state
        .cache
        .frame_generation
        .map(|_| (&state.cache.frame.status, &state.cache.frame.term))
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
        filter: None,
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
        state.ui.status = Some(crate::app::state::StatusMsg {
            text: "synced".into(),
            style: CellStyle::Normal,
            expires: chrono::DateTime::UNIX_EPOCH,
        });
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
                left: vec![
                    Cell {
                        text: "[TWO]".into(),
                        style: CellStyle::Primary,
                    },
                    Cell {
                        text: " 0 repos".into(),
                        style: CellStyle::Dim,
                    },
                ],
                center: Some(Cell {
                    text: "synced".into(),
                    style: CellStyle::Normal,
                }),
                right: vec![Cell {
                    text: "? help".into(),
                    style: CellStyle::Dim,
                }],
            }
        );
        assert_eq!(frame.prev_list, Some((empty_list(), Direction::Left)));
        assert_eq!(frame.term, term_status(&state));
    }

    #[test]
    fn empty_registry_gives_an_empty_list() {
        let tabs: Vec<Box<dyn Tab>> = Vec::new();
        let mut state = AppState::with_tabs(SIZE, &tabs);
        let frame = frame_model_with(&mut state, &tabs);
        assert_eq!(frame.list, empty_list());
        assert_eq!(frame.tab_bar.titles, [""; 0]);
    }

    #[test]
    fn status_shows_the_repo_count_and_the_cache_age() {
        let (tabs, mut state, _) = fake_tabs();
        state.data.session.repos = vec![crate::domain::repo::Repo {
            key: crate::domain::keys::RepoKey {
                owner: "o".into(),
                name: "r".into(),
            },
            path: None,
            settings: None,
        }];
        assert_eq!(status_model(&state, &tabs).left[1].text, " 1 repo");
        state.data.cache_at = Some(crate::domain::test_time());
        state.data.now = crate::domain::test_time() + chrono::TimeDelta::seconds(330);
        assert_eq!(
            status_model(&state, &tabs).left[1].text,
            " 1 repo · cache from 5m ago"
        );
        state.data.prefetch = Some(crate::app::state::PrefetchBatch { done: 1, total: 3 });
        assert_eq!(
            status_model(&state, &tabs).left[1].text,
            " 1 repo · cache from 5m ago · prefetch 2"
        );
    }
}
