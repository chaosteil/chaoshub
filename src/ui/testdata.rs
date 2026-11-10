//! Frame model fixtures and draw helpers for the ui tests.

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::app::layout::{Areas, layout};
use crate::app::models::{
    Align, Cell, CellStyle, Column, ColumnWidth, FrameModel, HelpModel, HelpSection, KeyHint,
    KeyHintsModel, ListModel, OverlayContent, OverlayModel, PaneModel, PaneTab, Row, RowItem,
    RowStyle, StatusModel, TabBarModel, TreeNode,
};
use crate::bus::{
    ConfirmSpec, KeyChord, KeyCode, KeyModifiers, OverlayKind, PrRef, Progress, PromptSpec, Size,
    TermState, TermStatus,
};
use crate::domain::keys::{PrKey, RepoKey};
use crate::theme::{PaletteChoice, Theme, ThemeKind, ThemeStyle};

/// The Pierre theme with grunge and textures on.
pub(crate) fn pierre() -> Theme {
    Theme::styled(
        ThemeKind::TrueColor,
        ThemeStyle::Grunge,
        PaletteChoice::Pierre,
        true,
        true,
    )
}

/// The grunge style on a 16-color terminal: the ANSI palette and the
/// plain skin.
pub(crate) fn ansi16() -> Theme {
    Theme::styled(
        ThemeKind::Ansi16,
        ThemeStyle::Grunge,
        PaletteChoice::Pierre,
        true,
        true,
    )
}

/// The punk theme on a true-color terminal.
pub(crate) fn punk() -> Theme {
    Theme::punk(ThemeKind::TrueColor)
}

/// The punk theme on a 16-color terminal.
pub(crate) fn punk_ansi16() -> Theme {
    Theme::punk(ThemeKind::Ansi16)
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
        column("flg", ColumnWidth::Fixed(4), Align::Left),
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

/// The list of the mine tab: a stack of three pull requests and one
/// single pull request.
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
        status: status(),
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

/// The key hints of the list.
pub(crate) fn hints() -> KeyHintsModel {
    let hint = |key: &str, label: &str, code: KeyCode| KeyHint {
        key: key.to_owned(),
        label: label.to_owned(),
        primary: true,
        chord: Some(KeyChord {
            code,
            mods: KeyModifiers::NONE,
        }),
    };
    KeyHintsModel {
        hints: vec![
            hint("j/k", "move", KeyCode::Char('j')),
            hint("Enter", "open", KeyCode::Enter),
            hint("v", "review", KeyCode::Char('v')),
            hint("m", "merge", KeyCode::Char('m')),
            hint("/", "filter", KeyCode::Char('/')),
            hint("?", "help", KeyCode::Char('?')),
        ],
    }
}

/// A frame model after a switch from the mine tab to the team tab.
pub(crate) fn sliding_frame() -> FrameModel {
    FrameModel {
        tab_bar: TabBarModel {
            titles: vec!["mine", "team"],
            active: 1,
        },
        list: team_list(),
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
                    title: "Description",
                    pane: 0,
                },
                PaneTab {
                    key: '2',
                    title: "Checks",
                    pane: 1,
                },
                PaneTab {
                    key: '3',
                    title: "Conversation",
                    pane: 2,
                },
                PaneTab {
                    key: '4',
                    title: "Stack",
                    pane: 3,
                },
            ],
            active: 1,
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
        content: OverlayContent::Help {
            help: HelpModel {
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
            },
            scroll: 0,
        },
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
            scroll: count.saturating_sub(1),
        },
    }
}

/// The confirm overlay of a stack merge.
pub(crate) fn confirm_overlay(danger: bool) -> OverlayModel {
    OverlayModel {
        kind: OverlayKind::Confirm,
        closing: false,
        content: OverlayContent::Confirm(ConfirmSpec {
            title: "merge #102".to_owned(),
            lines: vec![
                "Squash merge, merges 2 pull requests atomically:".to_owned(),
                "  1 #101 feat(app): Pane trait, registry and overlay stack  ✓ ✓".to_owned(),
                "  2 #102 feat(app): checks pane  ✓ ○".to_owned(),
                "warning: #102 merge state is not known yet".to_owned(),
            ],
            danger,
        }),
    }
}

/// The author picker with `count` authors and the cursor on `cursor`.
pub(crate) fn picker_overlay(count: usize, cursor: usize) -> OverlayModel {
    OverlayModel {
        kind: OverlayKind::Picker,
        closing: false,
        content: OverlayContent::Picker {
            title: "author".to_owned(),
            items: (0..count).map(|i| format!("colleague-{i:02}")).collect(),
            cursor,
        },
    }
}
