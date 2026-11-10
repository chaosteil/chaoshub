//! The Mine tab: the open pull requests of the user, as stacks.
//!
//! A stack shows a header row (`stack #S -> main · 3 PRs`) and its pull
//! requests top first with the tree glyphs `┌ ├ └`. A pull request
//! without a stack is a single row with `─`. The local columns come from
//! the matched local change. For a jj change they show the change id, the
//! commit and the workspace. For a git branch they show the branch name,
//! the short commit and the worktree. A list can show both.

use std::collections::HashMap;

use crate::app::filter::{Filter, ItemKey, pr_candidate};
use crate::app::models::{
    Align, Cell, CellStyle, Column, ColumnWidth, ListModel, Row, RowItem, RowStyle,
};
use crate::app::nav::place;
use crate::app::tabs::cells::{self, cell};
use crate::app::tabs::{Handled, KeyCx, Tab, TabState, list_interest, pr_ref};
use crate::bus::{Interest, KeyChord, Scope, Size, TargetKey};
use crate::domain::keys::PrKey;
use crate::domain::local::LocalMatch;
use crate::domain::matching::{group_stacks, match_prs};
use crate::domain::pr::{PrState, PullRequest};
use crate::domain::stack::StackGroup;

/// The minimum width of the title column.
const TITLE_MIN_WIDTH: u16 = 28;
/// From this list width on, the change id column is 12 wide.
const WIDE_FROM: u16 = 140;
/// Below this list width, the commit and workspace columns go away.
const NARROW_BELOW: u16 = 80;

/// The column set for a list width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Columns {
    /// Below `NARROW_BELOW`: no commit and no workspace column.
    Narrow,
    Normal,
    /// From `WIDE_FROM` on: a 12 cell change id.
    Wide,
}

impl Columns {
    fn for_width(width: u16) -> Self {
        if width >= WIDE_FROM {
            Self::Wide
        } else if width < NARROW_BELOW {
            Self::Narrow
        } else {
            Self::Normal
        }
    }

    /// The width of the change id column.
    fn change_width(self) -> u16 {
        if self == Self::Wide { 12 } else { 8 }
    }

    /// True when the commit and the workspace columns show.
    fn local_details(self) -> bool {
        self != Self::Narrow
    }
}

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
        TabState {
            ext: Box::new(Filter::default()),
            ..TabState::default()
        }
    }

    fn model(&self, cx: &KeyCx<'_>, ts: &TabState, size: Size) -> ListModel {
        let set = Columns::for_width(size.width);
        let filter = ts.ext.downcast_ref::<Filter>();
        let rows = rows(cx, set, filter);
        let (cursor, scroll) = place(
            &rows,
            ts.selected.as_ref(),
            ts.cursor,
            ts.scroll,
            usize::from(size.height),
        );
        ListModel {
            columns: columns(set),
            cursor,
            scroll,
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

    fn rescore(&self, cx: &KeyCx<'_>, ts: &mut TabState) {
        let Some(filter) = ts.ext.downcast_mut::<Filter>() else {
            return;
        };
        let candidates: Vec<_> = mine_prs(cx).into_iter().map(pr_candidate).collect();
        filter.rescore(&ts.filter, cx.data.revision, candidates);
    }

    fn authors(&self, _cx: &KeyCx<'_>) -> Vec<String> {
        Vec::new()
    }

    fn handle_key(&self, _key: KeyChord, _cx: &KeyCx<'_>, _ts: &mut TabState) -> Option<Handled> {
        None
    }

    fn help(&self) -> Vec<(KeyChord, &'static str)> {
        Vec::new()
    }
}

/// The columns: tree and title, number, change id, commit, workspace,
/// checks, reviews and flags. The narrow set has no commit and no workspace.
fn columns(set: Columns) -> Vec<Column> {
    let column = |title: &str, width, align| Column {
        title: title.into(),
        width,
        align,
    };
    let mut columns = vec![
        column(
            "title",
            ColumnWidth::Fill {
                min: TITLE_MIN_WIDTH,
            },
            Align::Left,
        ),
        column("#", ColumnWidth::Fixed(6), Align::Right),
        column(
            "change",
            ColumnWidth::Fixed(set.change_width()),
            Align::Left,
        ),
    ];
    if set.local_details() {
        columns.push(column("commit", ColumnWidth::Fixed(8), Align::Left));
        columns.push(column("workspace", ColumnWidth::Fixed(10), Align::Left));
    }
    columns.extend([
        column("checks", ColumnWidth::Fixed(7), Align::Center),
        column("rev", ColumnWidth::Fixed(5), Align::Center),
        column("flg", ColumnWidth::Fixed(4), Align::Left),
    ]);
    columns
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

/// The local view of the Mine tab: the local match of each pull request.
#[derive(Debug, Default)]
struct LocalView {
    matches: HashMap<PrKey, LocalMatch>,
}

/// Matches the scanned changes of each repo to the pull requests of that
/// repo.
fn local_view(cx: &KeyCx<'_>) -> LocalView {
    let mut view = LocalView::default();
    for (repo, scan) in &cx.data.local {
        let repo_prs: Vec<&PullRequest> = cx
            .data
            .prs
            .values()
            .filter(|pr| pr.key.repo == *repo)
            .collect();
        view.matches
            .extend(match_prs(&repo_prs, &scan.value.changes));
    }
    view
}

/// The rows of every stack group, of the pull requests that pass the
/// filter.
fn rows(cx: &KeyCx<'_>, set: Columns, filter: Option<&Filter>) -> Vec<Row> {
    let keeps = |key: ItemKey| filter.is_none_or(|filter| filter.keeps(&key));
    let mut prs = mine_prs(cx);
    prs.retain(|pr| keeps(ItemKey::Pr(pr.key.clone())));
    let view = local_view(cx);
    let mut rows = Vec::new();
    for group in group_stacks(&prs) {
        group_rows(cx, set, &group, &view, &mut rows);
    }
    rows
}

/// The rows of one group: a header and the tree for a stack, one row for
/// a single pull request.
fn group_rows(
    cx: &KeyCx<'_>,
    set: Columns,
    group: &StackGroup,
    view: &LocalView,
    rows: &mut Vec<Row>,
) {
    let prs: Vec<&PullRequest> = group
        .prs
        .iter()
        .filter_map(|key| cx.data.prs.get(key))
        .collect();
    if let Some(stack) = &group.stack {
        let noun = if stack.size == 1 { "PR" } else { "PRs" };
        rows.push(header(
            stack.id.clone(),
            format!(
                "stack #{} -> {} · {} {noun}",
                stack.number, stack.base_ref, stack.size
            ),
        ));
    }
    let last = prs.len().saturating_sub(1);
    for (i, pr) in prs.into_iter().enumerate() {
        let glyph = match i {
            _ if last == 0 => '─',
            0 => '┌',
            i if i == last => '└',
            _ => '├',
        };
        let local = view.matches.get(&pr.key).unwrap_or(&LocalMatch::None);
        let pane = cx.data.pane_open(&TargetKey::Pr(pr.key.clone()));
        rows.push(pr_row(pr, local, set, glyph, pane));
    }
}

/// A header row that spans the list.
fn header(stack_id: String, text: String) -> Row {
    Row {
        item: RowItem::StackHeader { stack_id },
        cells: vec![cell(text, CellStyle::Dim)],
        depth: 0,
        glyph: None,
        style: RowStyle::Header,
    }
}

/// The first `n` characters of an id.
fn short(id: &str, n: u16) -> String {
    id.chars().take(usize::from(n)).collect()
}

/// The change id, commit and workspace cells of a local match. For a git
/// branch, the change id is the branch name and the workspace is the
/// worktree.
fn local_cells(local: &LocalMatch, set: Columns) -> Vec<Cell> {
    let (change, commit, workspace, style) = match local {
        LocalMatch::None => (String::new(), String::new(), String::new(), CellStyle::Dim),
        LocalMatch::Synced {
            workspace,
            change_id,
            commit_id,
            ..
        } => (
            change_id.clone(),
            commit_id.clone(),
            workspace.clone(),
            CellStyle::Primary,
        ),
        LocalMatch::Diverged {
            workspace,
            change_id,
            commit_id,
            ..
        } => (
            change_id.clone(),
            commit_id.clone(),
            workspace.clone(),
            CellStyle::Warn,
        ),
    };
    let commit_style = if style == CellStyle::Warn {
        CellStyle::Warn
    } else {
        CellStyle::Dim
    };
    let mut cells = vec![cell(short(&change, set.change_width()), style)];
    if set.local_details() {
        cells.push(cell(short(&commit, 8), commit_style));
        cells.push(cell(workspace, CellStyle::Dim));
    }
    cells
}

/// The row of one pull request. `pane` is true while a pane of it is
/// open.
fn pr_row(pr: &PullRequest, local: &LocalMatch, set: Columns, glyph: char, pane: bool) -> Row {
    let head = &pr.head.value;
    let mut cells: Vec<Cell> = vec![
        cell(head.title.clone(), CellStyle::Normal),
        cell(format!("#{}", pr.key.number), CellStyle::Dim),
    ];
    cells.extend(local_cells(local, set));
    cells.extend([
        cells::checks(pr),
        cells::reviews(head),
        cells::flags(pr, pane),
    ]);
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
        single.checks.value = testdata::checks();
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
            keys: &crate::app::keymap::DEFAULT,
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
                cell("✗ 1/3", CellStyle::Danger),
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
    fn column_sets_follow_the_width() {
        let state = seeded();
        let areas = layout(SIZE);
        let cx = KeyCx {
            data: &state.data,
            target: None,
            areas: &areas,
            keys: &crate::app::keymap::DEFAULT,
        };
        let titles = |width| {
            let model = Mine.model(&cx, &TabState::default(), Size { width, height: 20 });
            for row in model
                .rows
                .iter()
                .filter(|row| matches!(row.item, RowItem::Pr(_)))
            {
                assert_eq!(row.cells.len(), model.columns.len(), "{width}");
            }
            model
                .columns
                .iter()
                .map(|c| (c.title.clone(), c.width))
                .collect::<Vec<_>>()
        };
        let change = |width| {
            titles(width)
                .into_iter()
                .find(|(title, _)| title == "change")
                .map(|(_, w)| w)
        };
        assert_eq!(change(100), Some(ColumnWidth::Fixed(8)));
        assert_eq!(change(139), Some(ColumnWidth::Fixed(8)));
        assert_eq!(change(140), Some(ColumnWidth::Fixed(12)));
        assert_eq!(titles(100).len(), 8);
        assert_eq!(titles(79).len(), 6, "no commit and no workspace");
    }

    #[test]
    fn the_filter_keeps_matching_prs_and_their_stack_header() {
        let state = seeded();
        let areas = layout(SIZE);
        let cx = KeyCx {
            data: &state.data,
            target: None,
            areas: &areas,
            keys: &crate::app::keymap::DEFAULT,
        };
        let mut ts = Mine.init_state();
        ts.filter = "middle".into();
        Mine.rescore(&cx, &mut ts);
        let model = Mine.model(&cx, &ts, SIZE);
        let expected: Vec<(Option<char>, String)> = vec![
            (None, "stack #12 -> main · 3 PRs".into()),
            (Some('─'), "Add the middle".into()),
        ];
        assert_eq!(outline(&model), expected);
        assert_eq!(model.cursor, 1, "the cursor skips the header");
    }

    /// The mine fixtures with the jj scan.
    fn with_local() -> AppState {
        let mut state = AppState::new(SIZE);
        state.load(testdata::mine_snapshot(), Vec::new());
        state
    }

    #[test]
    fn jj_columns() {
        let state = with_local();
        let model = model(&state, &Mine.init_state());
        let row = |title: &str| {
            model
                .rows
                .iter()
                .find(|row| row.cells[0].text == title)
                .unwrap()
                .clone()
        };
        let top = row("feat(ui): popup frame with pane tabs");
        assert_eq!(
            top.cells[2..5],
            [
                cell("wqnvzxop", CellStyle::Primary),
                cell("oid103", CellStyle::Dim),
                cell("default", CellStyle::Dim),
            ]
        );
        let middle = row("feat(app): checks pane");
        assert_eq!(
            middle.cells[2..5],
            [
                cell("xlmkrtpq", CellStyle::Warn),
                cell("9f9f9f9f", CellStyle::Warn),
                cell("default", CellStyle::Dim),
            ],
            "diverged by its bookmark"
        );
    }

    #[test]
    fn a_list_shows_jj_changes_and_git_branches() {
        use crate::domain::local::{LocalChange, LocalState, Vcs};

        let mut state = with_local();
        let mut export = pr("codeview", 3, "Fix the export");
        export.head.value.head_oid = "c0ffee0123456789".into();
        state.data.scopes.insert(export.key.clone(), Scope::Mine);
        state.data.prs.insert(export.key.clone(), export);
        let branch = LocalChange {
            workspace: "export".into(),
            change_id: "chaosteil/3".into(),
            commit_id: "c0ffee0123456789".into(),
            bookmarks: vec!["chaosteil/3".into()],
            empty: false,
            parents: Vec::new(),
            description: "Fix the export".into(),
            vcs: Vcs::Git,
        };
        state.data.local.insert(
            testdata::repo_key("codeview"),
            testdata::observed(LocalState {
                workspaces: Vec::new(),
                changes: vec![branch],
            }),
        );
        let model = model(&state, &Mine.init_state());
        let cells = |title: &str| {
            model
                .rows
                .iter()
                .find(|row| row.cells[0].text == title)
                .unwrap()
                .cells[2..5]
                .to_vec()
        };
        assert_eq!(
            cells("Fix the export"),
            [
                cell("chaostei", CellStyle::Primary),
                cell("c0ffee01", CellStyle::Dim),
                cell("export", CellStyle::Dim),
            ],
            "the branch name, the short commit and the worktree"
        );
        assert_eq!(
            cells("feat(ui): popup frame with pane tabs")[0],
            cell("wqnvzxop", CellStyle::Primary),
            "the jj change id"
        );
    }

    #[test]
    fn interest_names_the_cursor_row_and_the_visible_rows() {
        let state = seeded();
        let areas = layout(SIZE);
        let cx = KeyCx {
            data: &state.data,
            target: None,
            areas: &areas,
            keys: &crate::app::keymap::DEFAULT,
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
        assert_eq!(frame.tab_bar.titles, vec!["Mine", "Review", "Team"]);
        assert_eq!(frame.list.rows.len(), 5);
    }
}
