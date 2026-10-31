//! Frame model fixtures and draw helpers for the ui tests.

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::app::layout::{Areas, layout};
use crate::app::models::{
    Align, Cell, CellStyle, Column, ColumnWidth, Direction, FrameModel, HelpModel, HelpSection,
    ListModel, OverlayContent, OverlayModel, PaneModel, PaneTab, Row, RowItem, RowStyle,
    StatusModel, TabBarModel, TreeNode,
};
use crate::bus::{OverlayKind, PrRef, Progress, PromptSpec, Size, TermState, TermStatus};
use crate::domain::keys::{PrKey, RepoKey};
use crate::theme::{Theme, ThemeKind};

/// The Pierre theme with grunge and textures on.
pub(crate) fn pierre() -> Theme {
    Theme::new(ThemeKind::TrueColor, true, true)
}

/// The 16-color theme.
pub(crate) fn ansi16() -> Theme {
    Theme::new(ThemeKind::Ansi16, true, true)
}

/// The areas of a screen size.
pub(crate) fn areas(width: u16, height: u16) -> Areas {
    layout(Size { width, height })
}

/// A terminal with a test backend.
pub(crate) fn terminal(width: u16, height: u16) -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(width, height)).unwrap()
}

/// A cell.
pub(crate) fn cell(text: &str, style: CellStyle) -> Cell {
    Cell {
        text: text.to_owned(),
        style,
    }
}

fn column(title: &str, width: ColumnWidth, align: Align) -> Column {
    Column {
        title: title.to_owned(),
        width,
        align,
    }
}

fn pr_ref(number: u64) -> PrRef {
    PrRef {
        key: PrKey {
            repo: RepoKey {
                owner: "chaosteil".to_owned(),
                name: "chaoshub".to_owned(),
            },
            number,
        },
        node_id: format!("PR_{number}"),
        head_oid: format!("{number:040}"),
        base_ref: "main".to_owned(),
    }
}

/// The columns of the mine tab.
fn mine_columns() -> Vec<Column> {
    vec![
        column("title", ColumnWidth::Fill { min: 28 }, Align::Left),
        column("#", ColumnWidth::Fixed(6), Align::Right),
        column("change", ColumnWidth::Fixed(8), Align::Left),
        column("commit", ColumnWidth::Fixed(8), Align::Left),
        column("workspace", ColumnWidth::Fixed(10), Align::Left),
        column("ci", ColumnWidth::Fixed(3), Align::Center),
        column("rv", ColumnWidth::Fixed(3), Align::Center),
        column("flags", ColumnWidth::Fixed(4), Align::Left),
    ]
}

/// A pull request row of the mine tab.
#[allow(
    clippy::too_many_arguments,
    reason = "a fixture lists every column of a row"
)]
fn pr_row(
    number: u64,
    glyph: char,
    depth: u8,
    title: &str,
    change: &str,
    ci: (&str, CellStyle),
    review: (&str, CellStyle),
    flags: &str,
) -> Row {
    Row {
        item: RowItem::Pr(pr_ref(number)),
        cells: vec![
            cell(title, CellStyle::Normal),
            cell(&format!("#{number}"), CellStyle::Dim),
            cell(change, CellStyle::Primary),
            cell("3ccf7581", CellStyle::Dim),
            cell("default", CellStyle::Dim),
            cell(ci.0, ci.1),
            cell(review.0, review.1),
            cell(flags, CellStyle::Warn),
        ],
        depth,
        glyph: Some(glyph),
        style: RowStyle::Normal,
    }
}

/// The list of the mine tab: a stack of three pull requests, one single
/// pull request and one local-only change.
pub(crate) fn mine_list() -> ListModel {
    let header = Row {
        item: RowItem::StackHeader {
            stack_id: "S_1".to_owned(),
        },
        cells: vec![cell("stack #12 -> main · 3 PRs", CellStyle::Dim)],
        depth: 0,
        glyph: None,
        style: RowStyle::Header,
    };
    let local = Row {
        item: RowItem::Local {
            repo: pr_ref(0).key.repo,
            change_id: "zzmqxkrt".to_owned(),
        },
        cells: vec![
            cell("wip: try the new layout", CellStyle::Dim),
            cell("local", CellStyle::Dim),
            cell("zzmqxkrt", CellStyle::Local),
            cell("9a8b7c6d", CellStyle::Dim),
            cell("layout", CellStyle::Dim),
            cell("", CellStyle::Dim),
            cell("", CellStyle::Dim),
            cell("+", CellStyle::Local),
        ],
        depth: 1,
        glyph: Some('│'),
        style: RowStyle::LocalOnly,
    };
    ListModel {
        columns: mine_columns(),
        rows: vec![
            header,
            pr_row(
                103,
                '┌',
                0,
                "feat(ui): popup frame with pane tabs",
                "wqnvzxop",
                ("●", CellStyle::Warn),
                ("○", CellStyle::Dim),
                "D",
            ),
            local,
            pr_row(
                102,
                '├',
                0,
                "feat(app): checks pane",
                "xlmkrtpq",
                ("✓", CellStyle::Success),
                ("✓", CellStyle::Success),
                "",
            ),
            pr_row(
                101,
                '└',
                0,
                "feat(app): Pane trait, registry and overlay stack",
                "trptoqkq",
                ("✗", CellStyle::Danger),
                ("✗", CellStyle::Danger),
                "C",
            ),
            pr_row(
                99,
                '─',
                0,
                "fix(store): reopen the connection after an error",
                "kxyzmmop",
                ("·", CellStyle::Dim),
                ("·", CellStyle::Dim),
                "▶",
            ),
        ],
        cursor: 1,
        scroll: 0,
    }
}

/// The list of the team tab: two pull requests of colleagues.
pub(crate) fn team_list() -> ListModel {
    let columns = vec![
        column("", ColumnWidth::Fixed(2), Align::Left),
        column("title", ColumnWidth::Fill { min: 20 }, Align::Left),
        column("#", ColumnWidth::Fixed(6), Align::Right),
        column("repo", ColumnWidth::Fixed(12), Align::Left),
        column("author", ColumnWidth::Fixed(12), Align::Left),
    ];
    let row = |star: &str, title: &str, number: u64, author: &str| Row {
        item: RowItem::Pr(pr_ref(number)),
        cells: vec![
            cell(star, CellStyle::Warn),
            cell(title, CellStyle::Normal),
            cell(&format!("#{number}"), CellStyle::Dim),
            cell("chaoshub", CellStyle::Dim),
            cell(author, CellStyle::Info),
        ],
        depth: 0,
        glyph: None,
        style: RowStyle::Normal,
    };
    ListModel {
        columns,
        rows: vec![
            row("★", "docs(readme): keymap table", 88, "alice"),
            row("", "perf(store): batch inserts", 87, "bob"),
        ],
        cursor: 0,
        scroll: 0,
    }
}

/// The status bar with a message.
pub(crate) fn status() -> StatusModel {
    StatusModel {
        left: vec![
            cell("[MINE]", CellStyle::Primary),
            cell(" 3 repos · synced 12s ago · prefetch 2", CellStyle::Dim),
        ],
        center: Some(cell("merged #101", CellStyle::Success)),
        right: vec![cell("? help", CellStyle::Dim)],
    }
}

/// A frame model with the mine list and no overlay.
pub(crate) fn frame() -> FrameModel {
    FrameModel {
        tab_bar: TabBarModel {
            titles: vec!["mine", "team"],
            active: 0,
        },
        list: mine_list(),
        prev_list: None,
        status: status(),
        overlays: Vec::new(),
        term: TermStatus {
            state: TermState::Idle,
            msg: None,
            progress: Progress::Off,
        },
    }
}

/// A frame model during a slide from the mine tab to the team tab.
pub(crate) fn sliding_frame() -> FrameModel {
    FrameModel {
        tab_bar: TabBarModel {
            titles: vec!["mine", "team"],
            active: 1,
        },
        list: team_list(),
        prev_list: Some((mine_list(), Direction::Left)),
        ..frame()
    }
}

fn check_row(name: &str, state: (&str, CellStyle), duration: &str) -> Row {
    Row {
        item: RowItem::Pr(pr_ref(101)),
        cells: vec![
            cell(name, CellStyle::Normal),
            cell(state.0, state.1),
            cell(duration, CellStyle::Dim),
        ],
        depth: 0,
        glyph: None,
        style: RowStyle::Normal,
    }
}

/// The checks pane: a table of checks.
pub(crate) fn checks_pane() -> PaneModel {
    PaneModel::Table {
        columns: vec![
            column("name", ColumnWidth::Fill { min: 20 }, Align::Left),
            column("state", ColumnWidth::Fixed(10), Align::Left),
            column("duration", ColumnWidth::Fixed(8), Align::Right),
        ],
        rows: vec![
            check_row("ci / build", ("✓ success", CellStyle::Success), "2m 10s"),
            check_row("ci / test", ("✗ failure", CellStyle::Danger), "4m 02s"),
            check_row("ci / lint", ("● running", CellStyle::Warn), "48s"),
        ],
    }
}

/// A lines pane with a long line that wraps.
pub(crate) fn lines_pane() -> PaneModel {
    PaneModel::Lines {
        lines: vec![
            vec![
                cell("1 ", CellStyle::Dim),
                cell("#101 ", CellStyle::Primary),
                cell(
                    "feat(app): Pane trait, registry and overlay stack",
                    CellStyle::Normal,
                ),
            ],
            vec![
                cell("2 ", CellStyle::Dim),
                cell("#102 ", CellStyle::Primary),
                cell("feat(app): checks pane", CellStyle::Normal),
                cell(" stale", CellStyle::Warn),
            ],
        ],
        wrap: true,
    }
}

/// A tree pane: a comment, a collapsed thread and an open thread.
pub(crate) fn tree_pane() -> PaneModel {
    let node = |depth: u8, expanded: Option<bool>, cells: Vec<Cell>| TreeNode {
        depth,
        expanded,
        cells,
    };
    PaneModel::Tree {
        nodes: vec![
            node(
                0,
                None,
                vec![
                    cell("alice", CellStyle::Info),
                    cell(" · commented 2h ago", CellStyle::Dim),
                ],
            ),
            node(
                0,
                Some(false),
                vec![cell(
                    "src/ui/list.rs:42 · 3 comments · resolved",
                    CellStyle::Normal,
                )],
            ),
            node(
                0,
                Some(true),
                vec![cell("src/ui/pane.rs:7 · 1 comment", CellStyle::Normal)],
            ),
            node(
                1,
                None,
                vec![
                    cell("bob", CellStyle::Info),
                    cell(": the gutter eats one column", CellStyle::Normal),
                ],
            ),
        ],
    }
}

/// The PR popup overlay with the checks pane.
pub(crate) fn popup_overlay() -> OverlayModel {
    OverlayModel {
        kind: OverlayKind::Popup,
        closing: false,
        content: OverlayContent::Popup {
            title: "#101 feat(app): Pane trait, registry and overlay stack".to_owned(),
            subtitle: "chaosteil · chaosteil/push-trptoqkq -> main · open".to_owned(),
            tabs: vec![
                PaneTab {
                    key: '1',
                    title: "Checks",
                },
                PaneTab {
                    key: '2',
                    title: "Conversation",
                },
                PaneTab {
                    key: '3',
                    title: "Stack",
                },
            ],
            active: 0,
            pane: checks_pane(),
            cursor: 1,
            scroll: 0,
        },
    }
}

/// A frame model with one overlay over the mine list.
pub(crate) fn with_overlay(overlay: OverlayModel) -> FrameModel {
    FrameModel {
        overlays: vec![overlay],
        ..frame()
    }
}

/// The help overlay with the app keys and the actions.
pub(crate) fn help_overlay() -> OverlayModel {
    let entries = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(k, l)| ((*k).to_owned(), (*l).to_owned()))
            .collect()
    };
    OverlayModel {
        kind: OverlayKind::Help,
        closing: false,
        content: OverlayContent::Help(HelpModel {
            sections: vec![
                HelpSection {
                    title: "navigation".to_owned(),
                    entries: entries(&[
                        ("j / k", "move down / up"),
                        ("g / G", "first / last row"),
                        ("1 2 Tab", "switch tab"),
                        ("Enter", "open the popup"),
                        ("/", "filter"),
                    ]),
                },
                HelpSection {
                    title: "actions".to_owned(),
                    entries: entries(&[
                        ("v", "review in codeview"),
                        ("m", "merge"),
                        ("A", "approve"),
                        ("R", "request changes"),
                        ("x", "re-run failed checks"),
                    ]),
                },
            ],
        }),
    }
}

/// The prompt overlay of the request changes action.
pub(crate) fn prompt_overlay(text: &str) -> OverlayModel {
    OverlayModel {
        kind: OverlayKind::Prompt,
        closing: false,
        content: OverlayContent::Prompt {
            spec: PromptSpec {
                title: "request changes".to_owned(),
                placeholder: "What must change?".to_owned(),
            },
            text: text.to_owned(),
        },
    }
}

/// The output overlay with `count` numbered lines.
pub(crate) fn output_overlay(count: usize) -> OverlayModel {
    OverlayModel {
        kind: OverlayKind::Output,
        closing: false,
        content: OverlayContent::Output {
            title: "pr-agent review".to_owned(),
            lines: (1..=count)
                .map(|i| format!("{i:>3} | reviewing src/ui/list.rs hunk {i} of {count}"))
                .collect(),
        },
    }
}
