//! Builds the frame model from the app state.
//!
//! The frame model is rebuilt only when the generation moved. The list
//! model of each tab is cached with the generation and the size it was
//! built for, so animation frames and idle ticks never rebuild it.

use std::time::Duration;

use chrono::{DateTime, Utc};
use textwrap::core::display_width;

use crate::actions::{Action, Applicability, REGISTRY};
use crate::app::age::age;
use crate::app::keymap::{self, Keymap};
use crate::app::layout::{Areas, layout_full};
use crate::app::models::{
    Cell, CellStyle, FilterModel, FrameModel, HINT_CUT, HINT_GAP, HINT_PREFIX, KeyHint,
    KeyHintsModel, ListModel, OverlayContent, OverlayModel, PaneModel, PaneTab, StatusModel,
    TabBarModel, pane_tab_rows,
};
use crate::app::panes::conversation::is_stale;
use crate::app::panes::{PANES, Pane, applicable};
use crate::app::state::{AppData, AppState, OverlayState, PopupState};
use crate::app::tabs::{KeyCx, TABS, Tab};
use crate::app::target::{pr_target, row_target};
use crate::app::term::term_status;
use crate::bus::{
    KeyChord, KeyCode, KeyModifiers, Progress, ReviewKind, Scope, Selection, Size, Target,
    TermState, TermStatus,
};
use crate::domain::keys::PrKey;
use crate::domain::pr::PrState;

/// The frames of the spinner in the status bar.
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// How long one frame of the spinner shows. `run` ticks at this rate
/// while the app is busy.
pub(crate) const SPINNER_FRAME: Duration = Duration::from_millis(100);

/// The models of the last frame.
#[derive(Debug)]
pub(crate) struct ModelCache {
    /// The list model of each tab, by registry index.
    lists: Vec<Option<CachedList>>,
    /// The generation that `frame` was built for. `None` before the first
    /// build.
    frame_generation: Option<u64>,
    frame: FrameModel,
    /// The key bindings that the key hints show.
    keymap: Keymap,
    /// False when `[theme] key_hints = false`. The frame then has no key
    /// hints.
    key_hints: bool,
}

impl Default for ModelCache {
    /// No models, the default key bindings and the key hints on.
    fn default() -> Self {
        Self {
            lists: Vec::new(),
            frame_generation: None,
            frame: empty_frame(),
            keymap: keymap::DEFAULT.clone(),
            key_hints: true,
        }
    }
}

impl ModelCache {
    /// Sets the key bindings that the key hints show and the
    /// `[theme] key_hints` switch. The next frame is built again.
    pub(crate) fn set_key_hints(&mut self, keymap: Keymap, on: bool) {
        self.keymap = keymap;
        self.key_hints = on;
        self.frame_generation = None;
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
        let hints = hints_model(state, tabs, panes, &list);
        let titles = if state.ui.pinned.is_some() {
            Vec::new()
        } else {
            tabs.iter().map(|tab| tab.title()).collect()
        };
        let frame = FrameModel {
            tab_bar: TabBarModel {
                titles,
                active: state.ui.active,
            },
            list,
            status: status_model(state, tabs),
            overlays: overlay_models(state, panes, &REGISTRY),
            term: term_status(state),
            filter: filter_model(state),
            hints,
            column_headers: column_headers(state),
        };
        state.cache.frame = frame;
        state.cache.frame_generation = Some(state.generation);
    }
    &state.cache.frame
}

/// The models of the overlay stack, bottom first. The actions menu shows
/// the actions of `actions`.
fn overlay_models(
    state: &AppState,
    panes: &[Box<dyn Pane>],
    actions: &[Box<dyn Action>],
) -> Vec<OverlayModel> {
    let areas = areas(state);
    let pinned = state.ui.pinned.is_some();
    state
        .ui
        .overlays
        .iter()
        .map(|overlay| OverlayModel {
            kind: overlay.kind,
            closing: overlay.closing,
            content: match &overlay.state {
                OverlayState::Popup(popup) => {
                    popup_content(&state.data, &areas, pinned, popup, panes)
                }
                OverlayState::Authors(picker) => OverlayContent::Picker {
                    title: "author".into(),
                    items: picker.items(),
                    cursor: picker.cursor,
                },
                OverlayState::Actions(menu) => {
                    let Target::Pr { pr, .. } = &*menu.target;
                    OverlayContent::Picker {
                        title: format!("actions for #{}", pr.key.number),
                        items: menu.items(&state.data, &state.cache.keymap, actions),
                        cursor: menu.row(menu.cursor),
                    }
                }
                OverlayState::Confirm { spec, .. } => OverlayContent::Confirm(spec.clone()),
                OverlayState::Prompt { spec, text, .. } => OverlayContent::Prompt {
                    spec: spec.clone(),
                    text: text.clone(),
                },
                OverlayState::Review(review) => OverlayContent::Review(review.spec(&state.data)),
                OverlayState::ReviewText {
                    title, text, error, ..
                } => OverlayContent::ReviewText {
                    title: title.clone(),
                    text: text.clone(),
                    error: error.clone(),
                },
                OverlayState::Help { help, scroll } => OverlayContent::Help {
                    help: help.clone(),
                    scroll: *scroll,
                },
                OverlayState::Output {
                    title,
                    lines,
                    scroll,
                    ..
                } => OverlayContent::Output {
                    title: title.clone(),
                    lines: lines.clone(),
                    scroll: *scroll,
                },
            },
        })
        .collect()
}

/// The pane tabs for `target`, in registry order: the panes that apply,
/// and the dim tabs of the panes that keep their tab. When the tabs with
/// the full titles do not fit on one row of `width` cells, the tabs have
/// the short titles.
pub(crate) fn pane_tabs(panes: &[Box<dyn Pane>], target: &Target, width: u16) -> Vec<PaneTab> {
    let tabs = |short: bool| -> Vec<PaneTab> {
        panes
            .iter()
            .enumerate()
            .filter_map(|(i, pane)| {
                let applies = pane.applies(target);
                (applies || pane.keeps_tab()).then(|| PaneTab {
                    key: pane.key(),
                    title: if short {
                        pane.short_title()
                    } else {
                        pane.title()
                    },
                    pane: i,
                    dim: !applies,
                })
            })
            .collect()
    };
    let full = tabs(false);
    if pane_tab_rows(&full, width).len() > 1 {
        tabs(true)
    } else {
        full
    }
}

/// The size of the pane body of the popup with `tabs`. Each row of pane
/// tabs after the first takes one row from the body.
pub(crate) fn pane_size(areas: &Areas, tabs: &[PaneTab]) -> Size {
    let body = areas.popup_body;
    let more = pane_tab_rows(tabs, body.width).len().saturating_sub(1);
    Size {
        width: body.width,
        height: body
            .height
            .saturating_sub(u16::try_from(more).unwrap_or(u16::MAX)),
    }
}

/// The content of the pull request popup in `areas`: the header, the
/// tabs of the panes that apply, and the model of the active pane. A pull
/// request that is not loaded is `not loaded yet` in the pinned mode, and
/// gone in the list mode.
fn popup_content(
    data: &AppData,
    areas: &Areas,
    pinned: bool,
    popup: &PopupState,
    panes: &[Box<dyn Pane>],
) -> OverlayContent {
    let Some(target) = pr_target(data, &popup.key) else {
        let subtitle = if pinned {
            "not loaded yet"
        } else {
            "this pull request is gone"
        };
        return OverlayContent::Popup {
            title: format!("#{}", popup.key.number),
            subtitle: subtitle.into(),
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
    let Target::Pr { pr, .. } = &target;
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
    let tabs = pane_tabs(panes, &target, areas.popup_body.width);
    let cx = KeyCx {
        data,
        target: Some(&target),
        areas,
        keys: &keymap::DEFAULT,
    };
    let size = pane_size(areas, &tabs);
    let active = tabs
        .iter()
        .position(|tab| tab.pane == popup.active && !tab.dim)
        .unwrap_or(0);
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
        active,
        pane,
        cursor,
        scroll,
    }
}

/// The key hint bar of the active context: the top overlay that is not
/// closing, else the filter line while the user types the filter, else
/// the list. `None` when the key hints are off.
fn hints_model(
    state: &AppState,
    tabs: &[Box<dyn Tab>],
    panes: &[Box<dyn Pane>],
    list: &ListModel,
) -> Option<KeyHintsModel> {
    if !state.cache.key_hints {
        return None;
    }
    let keymap = &state.cache.keymap;
    let hints = match state.ui.overlays.iter().rev().find(|o| !o.closing) {
        Some(overlay) => overlay_hints(&state.data, keymap, panes, &overlay.state),
        None if state.ui.filter_edit => filter_hints(),
        None => list_hints(state, keymap, tabs, &REGISTRY, list),
    };
    Some(KeyHintsModel {
        hints: fit(hints, usize::from(state.data.size.width)),
    })
}

/// The hints of the list: the app keys, the keys of the active tab, and
/// when the cursor row has a target, the `actions` key and the direct
/// keys of `actions` that are enabled for it. The primary hints come
/// first.
fn list_hints(
    state: &AppState,
    keymap: &Keymap,
    tabs: &[Box<dyn Tab>],
    actions: &[Box<dyn Action>],
    list: &ListModel,
) -> Vec<KeyHint> {
    let tab_names: Vec<String> = (1..=tabs.len().min(9))
        .map(|n| format!("tab_{n}"))
        .chain(std::iter::once("next_tab".to_owned()))
        .collect();
    let tab_names: Vec<&str> = tab_names.iter().map(String::as_str).collect();
    let mut hints: Vec<KeyHint> = [
        app_hint(keymap, &["down", "up"], "move", true),
        app_hint(keymap, &["open"], "open", true),
        app_hint(keymap, &["filter"], "filter", false),
        app_hint(keymap, &tab_names, "tabs", false),
        app_hint(keymap, &["refresh"], "refresh", false),
        app_hint(keymap, &["help"], "help", false),
        app_hint(keymap, &["quit"], "quit", false),
        app_hint(keymap, &["column_headers"], "headers", false),
    ]
    .into_iter()
    .flatten()
    .collect();
    if let Some(tab) = tabs.get(state.ui.active) {
        hints.extend(tab.help().into_iter().map(|(chord, label)| KeyHint {
            key: key_text(chord),
            label: label.to_owned(),
            primary: false,
            chord: Some(chord),
        }));
    }
    if let Some(target) = list
        .rows
        .get(list.cursor)
        .and_then(|row| row_target(&state.data, &row.item))
    {
        hints.extend(app_hint(keymap, &["actions"], "actions", true));
        hints.extend(direct_hints(&state.data, keymap, actions, target));
    }
    hints.sort_by_key(|hint| !hint.primary);
    hints
}

/// The secondary hints of the direct action keys (see
/// `Keymap::detail_chords`) that are enabled for `target`, in registry
/// order. A hint shows the first chord of its action.
fn direct_hints(
    data: &AppData,
    keymap: &Keymap,
    actions: &[Box<dyn Action>],
    target: Target,
) -> Vec<KeyHint> {
    actions
        .iter()
        .zip(applicability(data, actions, target))
        .filter(|(_, applies)| *applies == Applicability::Enabled)
        .filter_map(|(action, _)| {
            let chord = keymap.detail_chords(action.id()).first().copied()?;
            Some(KeyHint {
                key: key_text(chord),
                label: action.id().replace('_', " "),
                primary: false,
                chord: Some(chord),
            })
        })
        .collect()
}

/// Whether each action of `actions` applies to `target`, in registry
/// order.
pub(crate) fn applicability(
    data: &AppData,
    actions: &[Box<dyn Action>],
    target: Target,
) -> Vec<Applicability> {
    let mut selection = Selection {
        target,
        me: data.session.login.clone().unwrap_or_default(),
        action_config: None,
    };
    actions
        .iter()
        .map(|action| {
            selection.action_config = data.action_config.get(action.id()).cloned();
            action.applies(&selection)
        })
        .collect()
}

/// The hints while the user types the filter: Enter keeps the filter,
/// Esc cancels it. The filter line reads these keys directly, not from
/// the key map.
fn filter_hints() -> Vec<KeyHint> {
    [
        hint(&[plain_key(KeyCode::Enter)], "apply"),
        hint(&[plain_key(KeyCode::Esc)], "cancel"),
    ]
    .into_iter()
    .flatten()
    .collect()
}

/// The chord of a key without modifiers.
fn plain_key(code: KeyCode) -> KeyChord {
    KeyChord {
        code,
        mods: KeyModifiers::NONE,
    }
}

/// The chord of the character `c` with `ctrl`.
fn ctrl_key(c: char) -> KeyChord {
    KeyChord {
        code: KeyCode::Char(c),
        mods: KeyModifiers::CONTROL,
    }
}

/// The hints of an overlay. They are all primary. The pull request
/// popup has no action hints: the `actions` hint opens the actions menu.
fn overlay_hints(
    data: &AppData,
    keymap: &Keymap,
    panes: &[Box<dyn Pane>],
    overlay: &OverlayState,
) -> Vec<KeyHint> {
    let hints = match overlay {
        OverlayState::Popup(popup) => {
            let target = pr_target(data, &popup.key);
            let mut keys: Vec<KeyChord> = target
                .as_ref()
                .map(|target| {
                    applicable(panes, target)
                        .into_iter()
                        .map(|i| plain_key(KeyCode::Char(panes[i].key())))
                        .collect()
                })
                .unwrap_or_default();
            keys.extend(first_chords(keymap, &["next_tab"]));
            vec![
                hint(&keys, "pane tabs"),
                app_hint(keymap, &["open"], "expand", true),
                app_hint(keymap, &["close_overlay"], "close", true),
                target.and_then(|_| app_hint(keymap, &["actions"], "actions", true)),
            ]
        }
        OverlayState::Confirm { .. } => vec![
            app_hint(keymap, &["confirm"], "confirm", true),
            app_hint(keymap, &["cancel", "close_overlay"], "cancel", true),
        ],
        OverlayState::Prompt { .. } => vec![
            hint(&[plain_key(KeyCode::Enter)], "submit"),
            hint(&[plain_key(KeyCode::Esc)], "cancel"),
        ],
        OverlayState::Review(_) => {
            let events: Vec<KeyChord> = ReviewKind::ALL
                .iter()
                .map(|event| plain_key(KeyCode::Char(event.key())))
                .collect();
            vec![
                app_hint(keymap, &["down", "up"], "move", true),
                app_hint(keymap, &["open"], "mark", true),
                hint(&[plain_key(KeyCode::Char('m'))], "message"),
                hint(&events, "send"),
                app_hint(keymap, &["close_overlay"], "cancel", true),
            ]
        }
        OverlayState::ReviewText { .. } => vec![
            hint(&[ctrl_key('s')], "done"),
            hint(&[ctrl_key('e')], "editor"),
            hint(&[plain_key(KeyCode::Esc)], "back"),
        ],
        OverlayState::Authors(_) => vec![
            app_hint(keymap, &["open"], "select", true),
            app_hint(keymap, &["close_overlay"], "close", true),
        ],
        OverlayState::Actions(_) => vec![
            app_hint(keymap, &["down", "up"], "move", true),
            app_hint(keymap, &["open"], "run", true),
            app_hint(keymap, &["close_overlay"], "close", true),
        ],
        OverlayState::Output { .. } | OverlayState::Help { .. } => {
            vec![app_hint(keymap, &["close_overlay"], "close", true)]
        }
    };
    hints.into_iter().flatten().collect()
}

/// The hint of the bindings `names`: the first chord of each binding,
/// joined with `/`. A click presses the first of these chords. `None`
/// when no binding has a chord.
fn app_hint(keymap: &Keymap, names: &[&str], label: &str, primary: bool) -> Option<KeyHint> {
    let chords = first_chords(keymap, names);
    (!chords.is_empty()).then(|| KeyHint {
        key: keys_text(&chords),
        label: label.to_owned(),
        primary,
        chord: chords.first().copied(),
    })
}

/// A primary hint of `chords`, joined with `/`. A click presses the
/// first chord. `None` when `chords` is empty.
fn hint(chords: &[KeyChord], label: &str) -> Option<KeyHint> {
    (!chords.is_empty()).then(|| KeyHint {
        key: keys_text(chords),
        label: label.to_owned(),
        primary: true,
        chord: chords.first().copied(),
    })
}

/// The first chord of each binding in `names` that has one.
fn first_chords(keymap: &Keymap, names: &[&str]) -> Vec<KeyChord> {
    names
        .iter()
        .filter_map(|name| keymap.chords(name).first().copied())
        .collect()
}

/// The text of `chords` in a hint, joined with `/`.
fn keys_text(chords: &[KeyChord]) -> String {
    chords
        .iter()
        .map(|&chord| key_text(chord))
        .collect::<Vec<_>>()
        .join("/")
}

/// The text of a chord in a hint: the text of the key map, with a
/// capital first letter when it is longer than one character, for
/// example `Enter` or `Ctrl-d`.
pub(crate) fn key_text(chord: KeyChord) -> String {
    let text = keymap::chord_text(chord);
    let mut chars = text.chars();
    match chars.next() {
        Some(first) if !chars.as_str().is_empty() => first.to_uppercase().chain(chars).collect(),
        _ => text,
    }
}

/// The display width of a hint: the key, a space and the label.
fn hint_width(hint: &KeyHint) -> usize {
    let space = usize::from(!hint.key.is_empty() && !hint.label.is_empty());
    display_width(&hint.key) + space + display_width(&hint.label)
}

/// The hints that fit `width` cells after `HINT_PREFIX`, with `HINT_GAP`
/// between them. The primary hints always stay. The secondary hints fill
/// the rest in order; after the first one that does not fit, the rest
/// is cut, and a last hint with the label `HINT_CUT` marks the cut.
fn fit(hints: Vec<KeyHint>, width: usize) -> Vec<KeyHint> {
    let gap = display_width(HINT_GAP);
    let cut_width = gap + display_width(HINT_CUT);
    // Each hint adds a gap and its width. The prefix takes the place of
    // the gap before the first hint.
    let mut used = display_width(HINT_PREFIX).saturating_sub(gap)
        + hints
            .iter()
            .filter(|hint| hint.primary)
            .map(|hint| gap + hint_width(hint))
            .sum::<usize>();
    let last_secondary = hints.iter().rposition(|hint| !hint.primary);
    let mut cut = false;
    let mut kept = Vec::with_capacity(hints.len());
    for (i, hint) in hints.into_iter().enumerate() {
        if hint.primary {
            kept.push(hint);
            continue;
        }
        if cut {
            continue;
        }
        let cost = gap + hint_width(&hint);
        let reserve = if Some(i) == last_secondary {
            0
        } else {
            cut_width
        };
        if used + cost + reserve <= width {
            used += cost;
            kept.push(hint);
        } else {
            cut = true;
        }
    }
    if cut {
        kept.push(KeyHint {
            key: String::new(),
            label: HINT_CUT.to_owned(),
            primary: false,
            chord: None,
        });
    }
    kept
}

/// The areas of the terminal size, with the key hint bar when the key
/// hints are on, for the list mode or the pinned mode.
pub(crate) fn areas(state: &AppState) -> Areas {
    layout_full(
        state.data.size,
        state.cache.key_hints,
        state.ui.pinned.is_some(),
    )
}

/// The size of the list area that the tab models fill: the list area
/// without the header row and the filter line when they show.
pub(crate) fn list_size(state: &AppState) -> Size {
    let areas = areas(state);
    let filter = u16::from(filter_model(state).is_some());
    let header = u16::from(column_headers(state));
    Size {
        width: areas.list.width,
        height: areas.list.height.saturating_sub(filter + header),
    }
}

/// True when the list shows the header row: the switch is on and the
/// app is not pinned (the pinned mode has no list).
fn column_headers(state: &AppState) -> bool {
    state.ui.column_headers && state.ui.pinned.is_none()
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

/// The list model of the active tab for the current generation. The
/// pinned mode has no list.
pub(crate) fn active_list(state: &mut AppState, tabs: &[Box<dyn Tab>]) -> ListModel {
    if state.ui.pinned.is_some() {
        return empty_list();
    }
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
    let areas = areas(state);
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
/// the left, the last message in the center, the work that runs and the
/// help hint on the right. In the pinned mode the left shows the pull
/// request, its repo and the age of its data.
pub(crate) fn status_model(state: &AppState, tabs: &[Box<dyn Tab>]) -> StatusModel {
    let left = if let Some(key) = &state.ui.pinned {
        pinned_status(&state.data, key)
    } else {
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
        left
    };
    let mut right = Vec::new();
    if let Some(label) = activity(&state.data) {
        right.push(Cell {
            text: spinner(state.data.now).to_owned(),
            style: CellStyle::Primary,
        });
        right.push(Cell {
            text: format!(" {label}   "),
            style: CellStyle::Normal,
        });
    }
    right.push(Cell {
        text: "? help".into(),
        style: CellStyle::Dim,
    });
    StatusModel {
        left,
        center: state.ui.status.as_ref().map(|msg| Cell {
            text: msg.text.clone(),
            style: msg.style,
        }),
        right,
    }
}

/// The left of the status bar in the pinned mode, for example `[#7]`
/// and ` owner/name · updated 2m ago`, or ` owner/name · not loaded yet`
/// before the pull request is loaded.
fn pinned_status(data: &AppData, key: &PrKey) -> Vec<Cell> {
    let repo = format!("{}/{}", key.repo.owner, key.repo.name);
    let text = match data.prs.get(key) {
        Some(pr) => format!(
            " {repo} · updated {} ago",
            age(pr.head.observed_at, data.now)
        ),
        None => format!(" {repo} · not loaded yet"),
    };
    vec![
        Cell {
            text: format!("[#{}]", key.number),
            style: CellStyle::Primary,
        },
        Cell {
            text,
            style: CellStyle::Dim,
        },
    ]
}

/// The frame of the spinner at the time `now`.
pub(crate) fn spinner(now: DateTime<Utc>) -> &'static str {
    let step = now.timestamp_millis() / i64::try_from(SPINNER_FRAME.as_millis()).unwrap_or(1);
    SPINNER[usize::try_from(step).unwrap_or(0) % SPINNER.len()]
}

/// The work that runs, for example `syncing team · prefetch 2`: the
/// merge of a pull request, the syncs, the pull requests that the
/// prefetch batch has left, the conversation loads, the files loads, and
/// one `agent review #7` item per pr-agent run. `None` when no work runs.
fn activity(data: &AppData) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(merge) = data.merging() {
        parts.push(format!("merging #{}", merge.number()));
    }
    let scopes: Vec<&str> = [(Scope::Mine, "mine"), (Scope::Team, "team")]
        .into_iter()
        .filter(|(scope, _)| data.sync.pending.contains(scope) || data.sync.running.contains(scope))
        .map(|(_, name)| name)
        .collect();
    if !scopes.is_empty() {
        parts.push(format!("syncing {}", scopes.join(" and ")));
    }
    if let Some(batch) = data.prefetch {
        parts.push(format!("prefetch {}", batch.total - batch.done));
    }
    match data.details_loading.len() {
        0 => {}
        1 => parts.push("loading 1 conversation".to_owned()),
        n => parts.push(format!("loading {n} conversations")),
    }
    match data.files_loading.len() {
        0 => {}
        1 => parts.push("loading 1 diff".to_owned()),
        n => parts.push(format!("loading {n} diffs")),
    }
    for (key, kind) in &data.agent_jobs {
        parts.push(format!("agent {} #{}", kind.name(), key.number));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// The repo count and the age of the data, for example
/// ` 3 repos · synced 12s ago`.
fn sync_text(state: &AppState, tabs: &[Box<dyn Tab>]) -> String {
    let data = &state.data;
    let repos = data.session.repos.len();
    let noun = if repos == 1 { "repo" } else { "repos" };
    let scope = tabs.get(state.ui.active).and_then(|tab| tab.scope());
    match (
        scope.and_then(|scope| data.synced.get(&scope)),
        data.cache_at,
    ) {
        (Some(&at), _) => format!(" {repos} {noun} · synced {} ago", age(at, data.now)),
        (None, Some(at)) => format!(" {repos} {noun} · cache from {} ago", age(at, data.now)),
        (None, None) => format!(" {repos} {noun}"),
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
        hints: None,
        column_headers: false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
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
    fn frame_has_the_tab_bar_and_the_status() {
        let (tabs, mut state, [one, two]) = fake_tabs();
        state.ui.active = 1;
        state.ui.status = Some(crate::app::state::StatusMsg {
            text: "synced".into(),
            style: CellStyle::Normal,
            expires: chrono::DateTime::UNIX_EPOCH,
        });
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

    /// The texts of the right part of the status bar.
    fn right(state: &AppState) -> Vec<String> {
        status_model(state, &TABS)
            .right
            .into_iter()
            .map(|cell| cell.text)
            .collect()
    }

    #[test]
    fn the_right_shows_the_work_that_runs_with_a_spinner() {
        let mut state = AppState::with_tabs(SIZE, &TABS);
        state.data.now = crate::domain::test_time();
        assert_eq!(right(&state), ["? help"]);

        state.data.sync.pending.insert(Scope::Mine);
        let first = right(&state);
        assert_eq!(first[1..], [" syncing mine   ", "? help"]);
        state.data.now += SPINNER_FRAME;
        assert_ne!(right(&state)[0], first[0], "the spinner moves");

        state.data.sync.pending.clear();
        state.data.sync.running.insert(Scope::Team);
        assert_eq!(right(&state)[1], " syncing team   ", "a timed sync shows");
        state.data.sync.running.insert(Scope::Mine);
        state.data.prefetch = Some(crate::app::state::PrefetchBatch { done: 1, total: 3 });
        assert_eq!(right(&state)[1], " syncing mine and team · prefetch 2   ");
        assert!(state.data.busy());

        state.data.sync.running.clear();
        state.data.prefetch = None;
        let key = |number| crate::app::testdata::pr_key("chaoshub", number);
        state.data.details_loading.insert(key(1));
        assert_eq!(right(&state)[1], " loading 1 conversation   ");
        state.data.details_loading.insert(key(2));
        assert_eq!(right(&state)[1], " loading 2 conversations   ");
        state.data.files_loading.insert(key(1));
        assert_eq!(
            right(&state)[1],
            " loading 2 conversations · loading 1 diff   "
        );
        state.data.details_loading.clear();
        state.data.files_loading.insert(key(2));
        assert_eq!(right(&state)[1], " loading 2 diffs   ");

        state.data.files_loading.clear();
        assert!(!state.data.busy());
        state
            .data
            .agent_jobs
            .insert((key(7), crate::domain::agent::ToolKind::Review));
        assert_eq!(right(&state)[1], " agent review #7   ");
        assert!(state.data.busy());
        state
            .data
            .agent_jobs
            .insert((key(7), crate::domain::agent::ToolKind::Ask));
        state
            .data
            .agent_jobs
            .insert((key(9), crate::domain::agent::ToolKind::Improve));
        assert_eq!(
            right(&state)[1],
            " agent review #7 · agent ask #7 · agent improve #9   "
        );
    }

    #[test]
    fn the_right_shows_a_running_merge() {
        use crate::app::state::MergeState;
        use crate::bus::{MergeProgress, MergeStep};
        let mut state = AppState::with_tabs(SIZE, &TABS);
        state.load(crate::app::testdata::mine_snapshot(), Vec::new());
        state.data.now = crate::domain::test_time();
        let key = crate::app::testdata::pr_key("chaoshub", 102);
        let mut merge = MergeState::new(key.clone(), state.data.now);
        state.data.merge = Some(merge.clone());
        assert_eq!(right(&state)[1], " merging #102   ");
        assert!(state.data.busy());

        merge.step = MergeStep::Entry {
            number: 101,
            index: 1,
            total: 2,
        };
        state.data.merge = Some(merge.clone());
        assert_eq!(
            right(&state)[1],
            " merging #101   ",
            "the entry that merges"
        );
        merge.step = MergeStep::Retarget { number: 102 };
        state.data.merge = Some(merge.clone());
        assert_eq!(right(&state)[1], " merging #102   ");

        merge.progress = MergeProgress::Merged { sha: "abc".into() };
        merge.done = true;
        state.data.merge = Some(merge.clone());
        assert_eq!(
            right(&state)[1],
            " merging #102   ",
            "the refresh is not in"
        );
        let pr = state.data.prs.get_mut(&key).unwrap();
        pr.head.value.state = PrState::Merged;
        assert_eq!(
            right(&state),
            ["? help"],
            "the pull request shows the merge"
        );
        assert!(!state.data.busy());

        merge.progress = MergeProgress::Failed("checks failing".into());
        merge.done = false;
        state.data.merge = Some(merge);
        assert_eq!(right(&state), ["? help"], "a failure ends the run");
    }

    #[test]
    fn the_list_hints_have_the_actions_key_and_only_the_direct_keys() {
        let mut state = AppState::new(SIZE);
        state.load(crate::app::testdata::mine_snapshot(), Vec::new());
        state.data.session.login = Some("chaosteil".into());
        let list = active_list(&mut state, &TABS);
        assert!(
            row_target(&state.data, &list.rows[list.cursor].item).is_some(),
            "the cursor row has a target"
        );
        let keymap = Keymap::default();
        let hints = list_hints(&state, &keymap, &TABS, &REGISTRY, &list);
        let actions = hints.iter().find(|hint| hint.label == "actions");
        assert_eq!(
            actions.map(|hint| (hint.key.as_str(), hint.primary)),
            Some(("Space", true))
        );
        assert!(
            !hints
                .iter()
                .any(|hint| hint.chord == keymap::parse_chord("T")),
            "show_hidden is only in the help"
        );
        let browser = hints.iter().find(|hint| hint.label == "browser");
        assert_eq!(
            browser.map(|hint| (hint.key.as_str(), hint.primary)),
            Some(("o", false)),
            "the direct key of browser is a secondary hint"
        );
        for action in REGISTRY.iter() {
            if keymap::DETAIL_ACTIONS.contains(&action.id()) {
                continue;
            }
            let label = action.id().replace('_', " ");
            assert!(
                !hints.iter().any(|hint| hint.label == label),
                "no hint of the action {}",
                action.id()
            );
        }
    }

    #[test]
    fn the_spinner_cycles_through_its_frames() {
        let start = crate::domain::test_time();
        let frames: Vec<&str> = (0..=SPINNER.len())
            .map(|step| spinner(start + SPINNER_FRAME * u32::try_from(step).unwrap()))
            .collect();
        assert_eq!(frames[0], frames[SPINNER.len()]);
        let unique: std::collections::HashSet<_> = frames.iter().collect();
        assert_eq!(unique.len(), SPINNER.len());
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
    }
}
