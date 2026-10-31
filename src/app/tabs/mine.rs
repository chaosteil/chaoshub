//! The Mine tab: the open pull requests of the user, as stacks.
//!
//! A stack shows a header row (`stack #S -> main · 3 PRs`) and its pull
//! requests top first with the tree glyphs `┌ ├ └`. A pull request
//! without a stack is a single row with `─`.

use crate::app::models::{
    Align, Cell, CellStyle, Column, ColumnWidth, ListModel, Row, RowItem, RowStyle,
};
use crate::app::tabs::cells::{self, cell};
use crate::app::tabs::{Handled, KeyCx, Tab, TabState, list_interest, pr_ref};
use crate::bus::{Interest, KeyChord, Scope, Size};
use crate::domain::matching::group_stacks;
use crate::domain::pr::{PrState, PullRequest};
use crate::domain::stack::StackGroup;

/// The minimum width of the title column.
const TITLE_MIN_WIDTH: u16 = 28;

/// The Mine tab.
pub(crate) struct Mine;

impl Tab for Mine {
    fn id(&self) -> &'static str {
        "mine"
    }

    fn title(&self) -> &'static str {
        "Mine"
    }

    fn scope(&self) -> Option<Scope> {
        Some(Scope::Mine)
    }

    fn wants_local(&self) -> bool {
        true
    }

    fn init_state(&self) -> TabState {
        TabState::default()
    }

    fn model(&self, cx: &KeyCx<'_>, ts: &TabState, _size: Size) -> ListModel {
        let rows = rows(cx);
        ListModel {
            columns: columns(),
            cursor: ts.cursor.min(rows.len().saturating_sub(1)),
            scroll: ts.scroll,
            rows,
        }
    }

    fn interest(&self, cx: &KeyCx<'_>, ts: &TabState) -> Interest {
        let list = cx.areas.list;
        let size = Size {
            width: list.width,
            height: list.height,
        };
        list_interest(self, &self.model(cx, ts, size), list.height)
    }

    fn handle_key(&self, _key: KeyChord, _cx: &KeyCx<'_>, _ts: &mut TabState) -> Option<Handled> {
        None
    }

    fn help(&self) -> Vec<(KeyChord, &'static str)> {
        Vec::new()
    }
}

/// The columns: tree and title, number, change id, commit, workspace,
/// CI, review and flags.
fn columns() -> Vec<Column> {
    let column = |title: &str, width, align| Column {
        title: title.into(),
        width,
        align,
    };
    vec![
        column(
            "title",
            ColumnWidth::Fill {
                min: TITLE_MIN_WIDTH,
            },
            Align::Left,
        ),
        column("#", ColumnWidth::Fixed(6), Align::Right),
        column("change", ColumnWidth::Fixed(8), Align::Left),
        column("commit", ColumnWidth::Fixed(8), Align::Left),
        column("workspace", ColumnWidth::Fixed(10), Align::Left),
        column("ci", ColumnWidth::Fixed(3), Align::Center),
        column("rv", ColumnWidth::Fixed(3), Align::Center),
        column("flags", ColumnWidth::Fixed(4), Align::Left),
    ]
}

/// The open pull requests of the Mine scope, newest update first.
fn mine_prs<'a>(cx: &KeyCx<'a>) -> Vec<&'a PullRequest> {
    let data = cx.data;
    let mut prs: Vec<&PullRequest> = data
        .prs
        .values()
        .filter(|pr| data.scopes.get(&pr.key) == Some(&Scope::Mine))
        .filter(|pr| pr.head.value.state == PrState::Open)
        .collect();
    prs.sort_by(|a, b| {
        b.head
            .value
            .updated_at
            .cmp(&a.head.value.updated_at)
            .then_with(|| a.key.cmp(&b.key))
    });
    prs
}

/// The rows of every stack group.
fn rows(cx: &KeyCx<'_>) -> Vec<Row> {
    let prs = mine_prs(cx);
    let mut rows = Vec::new();
    for group in group_stacks(&prs) {
        group_rows(cx, &group, &mut rows);
    }
    rows
}

/// The rows of one group: a header and the tree for a stack, one row for
/// a single pull request.
fn group_rows(cx: &KeyCx<'_>, group: &StackGroup, rows: &mut Vec<Row>) {
    let prs: Vec<&PullRequest> = group
        .prs
        .iter()
        .filter_map(|key| cx.data.prs.get(key))
        .collect();
    let Some(stack) = &group.stack else {
        rows.extend(prs.into_iter().map(|pr| pr_row(pr, '─')));
        return;
    };
    let noun = if stack.size == 1 { "PR" } else { "PRs" };
    rows.push(Row {
        item: RowItem::StackHeader {
            stack_id: stack.id.clone(),
        },
        cells: vec![cell(
            format!(
                "stack #{} -> {} · {} {noun}",
                stack.number, stack.base_ref, stack.size
            ),
            CellStyle::Dim,
        )],
        depth: 0,
        glyph: None,
        style: RowStyle::Header,
    });
    let last = prs.len().saturating_sub(1);
    for (i, pr) in prs.into_iter().enumerate() {
        let glyph = match i {
            _ if last == 0 => '─',
            0 => '┌',
            i if i == last => '└',
            _ => '├',
        };
        rows.push(pr_row(pr, glyph));
    }
}

/// The row of one pull request.
fn pr_row(pr: &PullRequest, glyph: char) -> Row {
    let head = &pr.head.value;
    let empty = || cell("", CellStyle::Dim);
    let cells: Vec<Cell> = vec![
        cell(head.title.clone(), CellStyle::Normal),
        cell(format!("#{}", pr.key.number), CellStyle::Dim),
        empty(),
        empty(),
        empty(),
        cells::ci(pr.ci_state.value),
        cells::review(head.review_decision),
        cells::flags(pr),
    ];
    Row {
        item: RowItem::Pr(pr_ref(pr)),
        cells,
        depth: 0,
        glyph: Some(glyph),
        style: RowStyle::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::layout::layout;
    use crate::app::state::AppState;
    use crate::app::testdata::{self, pr};
    use crate::domain::pr::{CiState, ReviewDecision};
    use crate::domain::stack::StackRef;

    const SIZE: Size = Size {
        width: 100,
        height: 30,
    };

    fn stacked(number: u64, position: u32, title: &str) -> PullRequest {
        let mut pr = pr("chaoshub", number, title);
        pr.head.value.stack = Some(StackRef {
            id: "S_1".into(),
            number: 12,
            size: 3,
            position,
            base_ref: "main".into(),
        });
        pr
    }

    /// A state with a 3-PR stack, a single PR, a closed PR and a team PR.
    fn seeded() -> AppState {
        let mut state = AppState::new(SIZE);
        let mut single = pr("codeview", 3, "Fix the export");
        single.head.value.updated_at += chrono::TimeDelta::hours(1);
        single.ci_state.value = CiState::Failure;
        single.head.value.review_decision = Some(ReviewDecision::Approved);
        single.head.value.is_draft = true;
        let mut closed = pr("chaoshub", 4, "Closed");
        closed.head.value.state = PrState::Closed;
        let mine = [
            stacked(11, 1, "Add the base"),
            stacked(13, 3, "Add the top"),
            stacked(12, 2, "Add the middle"),
            single,
            closed,
        ];
        for pr in mine {
            state.data.scopes.insert(pr.key.clone(), Scope::Mine);
            state.data.prs.insert(pr.key.clone(), pr);
        }
        let team = pr("chaoshub", 50, "Theirs");
        state.data.scopes.insert(team.key.clone(), Scope::Team);
        state.data.prs.insert(team.key.clone(), team);
        state
    }

    fn model(state: &AppState, ts: &TabState) -> ListModel {
        let areas = layout(SIZE);
        let cx = KeyCx {
            data: &state.data,
            target: None,
            areas: &areas,
        };
        Mine.model(&cx, ts, SIZE)
    }

    /// The glyph and the first cell of each row.
    fn outline(model: &ListModel) -> Vec<(Option<char>, String)> {
        model
            .rows
            .iter()
            .map(|row| (row.glyph, row.cells[0].text.clone()))
            .collect()
    }

    #[test]
    fn stacks_render_as_trees_and_singles_as_rows() {
        let state = seeded();
        let model = model(&state, &TabState::default());
        let expected: Vec<(Option<char>, String)> = vec![
            (Some('─'), "Fix the export".into()),
            (None, "stack #12 -> main · 3 PRs".into()),
            (Some('┌'), "Add the top".into()),
            (Some('├'), "Add the middle".into()),
            (Some('└'), "Add the base".into()),
        ];
        assert_eq!(outline(&model), expected);
        assert_eq!(model.columns.len(), 8);
        assert_eq!(model.rows[1].style, RowStyle::Header);
        assert_eq!(
            model.rows[1].item,
            RowItem::StackHeader {
                stack_id: "S_1".into()
            }
        );
    }

    #[test]
    fn a_pr_row_has_every_column() {
        let state = seeded();
        let model = model(&state, &TabState::default());
        let single = &model.rows[0];
        assert_eq!(
            single.cells,
            vec![
                cell("Fix the export", CellStyle::Normal),
                cell("#3", CellStyle::Dim),
                cell("", CellStyle::Dim),
                cell("", CellStyle::Dim),
                cell("", CellStyle::Dim),
                cell("✗", CellStyle::Danger),
                cell("✓", CellStyle::Success),
                cell("D", CellStyle::Warn),
            ]
        );
        assert_eq!(
            single.item,
            RowItem::Pr(pr_ref(&state.data.prs[&testdata::pr_key("codeview", 3)]))
        );
    }

    #[test]
    fn a_stack_of_one_loaded_pr_uses_the_single_glyph() {
        let mut state = AppState::new(SIZE);
        let top = stacked(13, 3, "Add the top");
        state.data.scopes.insert(top.key.clone(), Scope::Mine);
        state.data.prs.insert(top.key.clone(), top);
        let model = model(&state, &TabState::default());
        assert_eq!(model.rows.len(), 2);
        assert_eq!(model.rows[1].glyph, Some('─'));
    }

    #[test]
    fn interest_names_the_cursor_row_and_the_visible_rows() {
        let state = seeded();
        let areas = layout(SIZE);
        let cx = KeyCx {
            data: &state.data,
            target: None,
            areas: &areas,
        };
        let ts = TabState {
            cursor: 2,
            ..TabState::default()
        };
        let interest = Mine.interest(&cx, &ts);
        let key = |n| pr_ref(&state.data.prs[&testdata::pr_key("chaoshub", n)]);
        assert_eq!(interest.tab, "mine");
        assert_eq!(interest.scope, Some(Scope::Mine));
        assert!(interest.wants_local);
        assert_eq!(interest.selected, Some(key(13)));
        assert_eq!(
            interest.visible.len(),
            4,
            "the header is not a pull request"
        );
    }

    #[test]
    fn the_frame_shows_the_mine_tab() {
        let mut state = seeded();
        let frame = crate::app::frame::frame_model(&mut state);
        assert_eq!(frame.tab_bar.titles, vec!["Mine"]);
        assert_eq!(frame.list.rows.len(), 5);
    }
}
