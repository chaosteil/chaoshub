//! Frame model fixtures and draw helpers for the ui tests.

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::app::layout::{Areas, layout};
use crate::app::models::{
    Align, Cell, CellStyle, Column, ColumnWidth, Direction, FrameModel, ListModel, Row, RowItem,
    RowStyle, StatusModel, TabBarModel,
};
use crate::bus::{PrRef, Progress, Size, TermState, TermStatus};
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
