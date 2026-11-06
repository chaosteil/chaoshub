//! Plain render models for `ui`. No ratatui types: `ui` maps the semantic
//! styles to theme colors.
#![allow(dead_code, reason = "app and ui build on the models in later phases")]

use crate::bus::{ConfirmSpec, KeyChord, OverlayKind, PrRef, PromptSpec, TermStatus};

/// Everything that `ui::view` draws in one frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrameModel {
    pub tab_bar: TabBarModel,
    pub list: ListModel,
    /// The list of the previous tab while a tab slide runs, and the
    /// direction of the slide.
    pub prev_list: Option<(ListModel, Direction)>,
    pub status: StatusModel,
    /// The overlay stack, bottom first.
    pub overlays: Vec<OverlayModel>,
    pub term: TermStatus,
    /// The filter line of the active tab, while the user edits it or
    /// while it has text. It takes the last row of the list area.
    pub filter: Option<FilterModel>,
    /// The key hint bar. `None` when `[theme] key_hints = false`.
    pub hints: Option<KeyHintsModel>,
}

/// The text before the first key hint: the stencil marker of the
/// headers and one space.
pub(crate) const HINT_PREFIX: &str = "▌ ";
/// The space between two key hints.
pub(crate) const HINT_GAP: &str = "  ";
/// The label of the last hint when the bar cut hints.
pub(crate) const HINT_CUT: &str = "…";

/// The key hint bar: the hints that fit the width, in display order.
/// When hints were cut, the last hint has no key and the label
/// `HINT_CUT`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyHintsModel {
    pub hints: Vec<KeyHint>,
}

/// One key hint, for example `v review`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KeyHint {
    /// The keys as the user configured them, for example `j/k`.
    pub key: String,
    pub label: String,
    /// True when the hint always shows.
    pub primary: bool,
    /// The first chord of the hint. A click on the hint presses it.
    /// `None` for the cut marker.
    pub chord: Option<KeyChord>,
}

/// The filter line: the filter text and whether the user edits it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FilterModel {
    pub text: String,
    pub editing: bool,
}

/// The tab bar: the tab titles in registry order and the active tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TabBarModel {
    pub titles: Vec<&'static str>,
    pub active: usize,
}

/// The direction of a slide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Direction {
    Left,
    Right,
}

/// A list of rows with columns, a cursor and a scroll offset. Any tab
/// produces one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListModel {
    pub columns: Vec<Column>,
    pub rows: Vec<Row>,
    pub cursor: usize,
    pub scroll: usize,
}

/// A column of a list or a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Column {
    pub title: String,
    pub width: ColumnWidth,
    pub align: Align,
}

/// The width of a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColumnWidth {
    /// A fixed number of cells.
    Fixed(u16),
    /// The rest of the width, with a minimum.
    Fill { min: u16 },
}

/// The alignment of the text in a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Align {
    Left,
    Right,
    Center,
}

/// One row of a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    pub item: RowItem,
    /// One cell per column.
    pub cells: Vec<Cell>,
    /// The depth in the stack tree.
    pub depth: u8,
    /// The tree glyph, for example `┌`.
    pub glyph: Option<char>,
    pub style: RowStyle,
}

/// What a row shows. `update` turns the selected item into a `Target`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RowItem {
    Pr(PrRef),
    StackHeader { stack_id: String },
    Separator,
}

/// A text with a semantic style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cell {
    pub text: String,
    pub style: CellStyle,
}

/// The semantic style of a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CellStyle {
    Normal,
    Dim,
    Primary,
    Success,
    Danger,
    Warn,
    Info,
    Merge,
    /// A comment body in markdown. `ui` renders it through tui-markdown
    /// and wraps it to the width of the pane.
    Markdown,
}

/// The semantic style of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RowStyle {
    Normal,
    Dim,
    Header,
    Selected,
}

/// The content of a popup pane. `ui` renders any pane model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PaneModel {
    Table {
        columns: Vec<Column>,
        rows: Vec<Row>,
    },
    Lines {
        /// Each line is a sequence of cells.
        lines: Vec<Vec<Cell>>,
        wrap: bool,
    },
    Tree {
        nodes: Vec<TreeNode>,
    },
}

/// One node of a pane tree, flattened in display order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TreeNode {
    pub depth: u8,
    /// `None` for a leaf.
    pub expanded: Option<bool>,
    pub cells: Vec<Cell>,
}

/// One overlay of the overlay stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OverlayModel {
    pub kind: OverlayKind,
    /// True while the overlay slides out.
    pub closing: bool,
    pub content: OverlayContent,
}

/// The content of an overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OverlayContent {
    Popup {
        title: String,
        subtitle: String,
        /// The panes that apply, in registry order.
        tabs: Vec<PaneTab>,
        active: usize,
        pane: PaneModel,
        cursor: usize,
        scroll: usize,
    },
    Confirm(ConfirmSpec),
    Prompt {
        spec: PromptSpec,
        text: String,
    },
    Output {
        title: String,
        lines: Vec<String>,
        /// The index of the first visible line. `ui` shows the last page
        /// when the index is past it.
        scroll: usize,
    },
    Picker {
        title: String,
        items: Vec<String>,
        cursor: usize,
    },
    Help {
        help: HelpModel,
        /// The index of the first visible line. `ui` shows the last page
        /// when the index is past it.
        scroll: usize,
    },
}

/// A pane tab in the popup header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneTab {
    pub key: char,
    pub title: &'static str,
}

/// The space between two pane tabs on one row. With the edge space of a
/// tab, two tabs of the plain skins have two spaces between them.
pub(crate) const PANE_TAB_GAP: &str = " ";

/// The cells of a pane tab next to its label. The skins draw one or two
/// edge cells, and `ui` fills a tab with one edge cell with a space. So
/// `app` and `ui` get the same rows for each skin.
const PANE_TAB_EDGES: usize = 2;

impl PaneTab {
    /// The text of the tab: ` 1 Description `.
    pub(crate) fn label(&self) -> String {
        format!(" {} {} ", self.key, self.title)
    }

    /// The width of the tab in cells: the label and the edges.
    pub(crate) fn cells(&self) -> usize {
        self.label().chars().count() + PANE_TAB_EDGES
    }
}

/// The pane tabs in rows of at most `width` cells: the index range of
/// the tabs of each row. A tab that is wider than `width` gets a row of
/// its own. The result has one row or more.
pub(crate) fn pane_tab_rows(tabs: &[PaneTab], width: u16) -> Vec<std::ops::Range<usize>> {
    let width = usize::from(width);
    let gap = PANE_TAB_GAP.len();
    let mut rows = Vec::new();
    let mut start = 0;
    let mut used = 0;
    for (i, tab) in tabs.iter().enumerate() {
        let cells = tab.cells();
        if i > start && used + gap + cells > width {
            rows.push(start..i);
            start = i;
            used = 0;
        }
        used += if i > start { gap + cells } else { cells };
    }
    rows.push(start..tabs.len());
    rows
}

/// The help overlay: key bindings in sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HelpModel {
    pub sections: Vec<HelpSection>,
}

impl HelpModel {
    /// The number of lines that the help overlay draws: a title line per
    /// section, a line per entry, and a blank line between two sections.
    pub(crate) fn line_count(&self) -> usize {
        let lines: usize = self.sections.iter().map(|s| 1 + s.entries.len()).sum();
        lines + self.sections.len().saturating_sub(1)
    }
}

/// One section of the help overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HelpSection {
    pub title: String,
    /// The key text and the label of each binding.
    pub entries: Vec<(String, String)>,
}

/// The status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StatusModel {
    pub left: Vec<Cell>,
    /// The last message.
    pub center: Option<Cell>,
    pub right: Vec<Cell>,
}
