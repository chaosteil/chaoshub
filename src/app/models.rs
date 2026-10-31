//! Plain render models for `ui`. No ratatui types: `ui` maps the semantic
//! styles to theme colors.
#![allow(dead_code, reason = "app and ui build on the models in later phases")]

use crate::bus::{ConfirmSpec, OverlayKind, PrRef, PromptSpec, TermStatus};
use crate::domain::keys::RepoKey;

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
    Local { repo: RepoKey, change_id: String },
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
    /// The marker of a local-only change.
    Local,
    /// A comment body in markdown. `ui` renders it through tui-markdown
    /// and wraps it to the width of the pane.
    Markdown,
}

/// The semantic style of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RowStyle {
    Normal,
    Dim,
    LocalOnly,
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
    },
    Picker {
        title: String,
        items: Vec<String>,
        cursor: usize,
    },
    Help(HelpModel),
}

/// A pane tab in the popup header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaneTab {
    pub key: char,
    pub title: &'static str,
}

/// The help overlay: key bindings in sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HelpModel {
    pub sections: Vec<HelpSection>,
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
