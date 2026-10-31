//! The Mine tab: the open pull requests of the user, as stacks.
//!
//! A stack shows a header row (`stack #S -> main · 3 PRs`) and its pull
//! requests top first with the tree glyphs `┌ ├ └`. A pull request
//! without a stack is a single row with `─`. The jj columns come from the
//! matched local change. A local-only change shows right above the pull
//! request of its nearest matched ancestor, else in a `local` group per
//! workspace at the end.

use std::collections::HashMap;

use crate::app::filter::{Filter, ItemKey, local_candidate, pr_candidate};
use crate::app::models::{
    Align, Cell, CellStyle, Column, ColumnWidth, ListModel, Row, RowItem, RowStyle,
};
use crate::app::nav::place;
use crate::app::tabs::cells::{self, cell};
use crate::app::tabs::{Handled, KeyCx, Tab, TabState, list_interest, pr_ref};
use crate::bus::{Interest, KeyChord, Scope, Size};
use crate::domain::keys::{PrKey, RepoKey};
use crate::domain::local::{LocalChange, LocalMatch};
use crate::domain::matching::{group_stacks, local_only, match_prs, nearest_pr};
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
        let prs = mine_prs(cx);
        let view = local_view(cx, &prs);
        let locals = view
            .anchored
            .values()
            .flatten()
            .chain(&view.loose)
            .map(|(repo, change)| local_candidate(repo, change));
        let candidates: Vec<_> = prs.into_iter().map(pr_candidate).chain(locals).collect();
        filter.rescore(&ts.filter, cx.data.revision, candidates);
    }

    fn handle_key(&self, _key: KeyChord, _cx: &KeyCx<'_>, _ts: &mut TabState) -> Option<Handled> {
        None
    }

    fn help(&self) -> Vec<(KeyChord, &'static str)> {
        Vec::new()
    }
}

/// The columns: tree and title, number, change id, commit, workspace,
/// CI, review and flags. The narrow set has no commit and no workspace.
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
        column("ci", ColumnWidth::Fixed(3), Align::Center),
        column("rv", ColumnWidth::Fixed(3), Align::Center),
        column("flags", ColumnWidth::Fixed(4), Align::Left),
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

/// The jj view of the Mine tab: the local match of each pull request, and
/// the local-only changes under a pull request or loose.
#[derive(Debug, Default)]
struct LocalView {
    matches: HashMap<PrKey, LocalMatch>,
    /// The local-only changes whose nearest matched ancestor is a pull
    /// request of the list, by that pull request.
    anchored: HashMap<PrKey, Vec<(RepoKey, LocalChange)>>,
    /// The other local-only changes, by repo and scan order.
    loose: Vec<(RepoKey, LocalChange)>,
}

/// Matches the scanned changes of each repo to the pull requests of that
/// repo, and places the local-only changes. `shown` are the pull requests
/// of the list; a change under another pull request is loose.
fn local_view(cx: &KeyCx<'_>, shown: &[&PullRequest]) -> LocalView {
    let mut view = LocalView::default();
    for (repo, scan) in &cx.data.local {
        let changes = &scan.value.changes;
        let repo_prs: Vec<&PullRequest> = cx
            .data
            .prs
            .values()
            .filter(|pr| pr.key.repo == *repo)
            .collect();
        let matches = match_prs(&repo_prs, changes);
        for change in local_only(changes, &matches) {
            let anchor = nearest_pr(&change, changes, &matches)
                .filter(|key| shown.iter().any(|pr| pr.key == *key));
            match anchor {
                Some(key) => view
                    .anchored
                    .entry(key)
                    .or_default()
                    .push((repo.clone(), change)),
                None => view.loose.push((repo.clone(), change)),
            }
        }
        view.matches.extend(matches);
    }
    view
}

/// The rows of every stack group, of the pull requests and local changes
/// that pass the filter. The loose local changes come last, one group per
/// workspace.
fn rows(cx: &KeyCx<'_>, set: Columns, filter: Option<&Filter>) -> Vec<Row> {
    let keeps = |key: ItemKey| filter.is_none_or(|filter| filter.keeps(&key));
    let mut prs = mine_prs(cx);
    prs.retain(|pr| keeps(ItemKey::Pr(pr.key.clone())));
    let mut view = local_view(cx, &prs);
    let keeps_local = |(repo, change): &(RepoKey, LocalChange)| {
        keeps(ItemKey::Local {
            repo: repo.clone(),
            change_id: change.change_id.clone(),
        })
    };
    for changes in view.anchored.values_mut() {
        changes.retain(keeps_local);
    }
    view.loose.retain(keeps_local);
    let mut rows = Vec::new();
    for group in group_stacks(&prs) {
        group_rows(cx, set, &group, &view, &mut rows);
    }
    loose_rows(set, &view.loose, &mut rows);
    rows
}

/// The rows of one group: a header and the tree for a stack, one row for
/// a single pull request. The local changes under a pull request come
/// right above its row.
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
        for (repo, change) in view.anchored.get(&pr.key).into_iter().flatten() {
            rows.push(local_row(repo, change, set, 1, '│'));
        }
        let local = view.matches.get(&pr.key).unwrap_or(&LocalMatch::None);
        rows.push(pr_row(pr, local, set, glyph));
    }
}

/// The loose local changes: a `local` header per repo and workspace, then
/// the changes.
fn loose_rows(set: Columns, loose: &[(RepoKey, LocalChange)], rows: &mut Vec<Row>) {
    let mut current: Option<(&RepoKey, &str)> = None;
    for (repo, change) in loose {
        let group = (repo, change.workspace.as_str());
        if current != Some(group) {
            current = Some(group);
            rows.push(header(
                format!("local:{}/{}:{}", repo.owner, repo.name, change.workspace),
                format!("local · {} · {}", repo.name, change.workspace),
            ));
        }
        rows.push(local_row(repo, change, set, 0, '─'));
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

/// The change id, commit and workspace cells of a local match.
fn local_cells(local: &LocalMatch, set: Columns) -> Vec<Cell> {
    let (change, commit, workspace, style) = match local {
        LocalMatch::None => (String::new(), String::new(), String::new(), CellStyle::Dim),
        LocalMatch::Synced {
            workspace,
            change_id,
            commit_id,
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

/// The row of one pull request.
fn pr_row(pr: &PullRequest, local: &LocalMatch, set: Columns, glyph: char) -> Row {
    let head = &pr.head.value;
    let mut cells: Vec<Cell> = vec![
        cell(head.title.clone(), CellStyle::Normal),
        cell(format!("#{}", pr.key.number), CellStyle::Dim),
    ];
    cells.extend(local_cells(local, set));
    cells.extend([
        cells::ci(pr.ci_state.value),
        cells::review(head.review_decision),
        cells::flags(pr),
    ]);
    Row {
        item: RowItem::Pr(pr_ref(pr)),
        cells,
        depth: 0,
        glyph: Some(glyph),
        style: RowStyle::Normal,
    }
}

/// The row of a local-only change: grey text, `local` in the number
/// column, and the orange `+` marker.
fn local_row(repo: &RepoKey, change: &LocalChange, set: Columns, depth: u8, glyph: char) -> Row {
    let title = if change.description.is_empty() {
        "(no description)".to_owned()
    } else {
        change.description.clone()
    };
    let mut cells = vec![
        cell(title, CellStyle::Dim),
        cell("local", CellStyle::Dim),
        cell(
            short(&change.change_id, set.change_width()),
            CellStyle::Local,
        ),
    ];
    if set.local_details() {
        cells.push(cell(short(&change.commit_id, 8), CellStyle::Dim));
        cells.push(cell(change.workspace.clone(), CellStyle::Dim));
    }
    cells.extend([
        cell("", CellStyle::Dim),
        cell("", CellStyle::Dim),
        cell("+", CellStyle::Local),
    ]);
    Row {
        item: RowItem::Local {
            repo: repo.clone(),
            change_id: change.change_id.clone(),
        },
        cells,
        depth,
        glyph: Some(glyph),
        style: RowStyle::LocalOnly,
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
    fn jj_columns_and_local_only_rows() {
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
        let wip = row("wip: try the new layout");
        assert_eq!(
            wip.item,
            RowItem::Local {
                repo: testdata::repo_key("chaoshub"),
                change_id: "zzmqxkrtzzmq".into(),
            }
        );
        assert_eq!(
            (wip.depth, wip.glyph, wip.style),
            (1, Some('│'), RowStyle::LocalOnly)
        );
        assert_eq!(wip.cells[1], cell("local", CellStyle::Dim));
        assert_eq!(wip.cells[2], cell("zzmqxkrt", CellStyle::Local));
        assert_eq!(wip.cells[7], cell("+", CellStyle::Local));
        let titles: Vec<&str> = model
            .rows
            .iter()
            .map(|r| r.cells[0].text.as_str())
            .collect();
        let wip_at = titles.iter().position(|t| t.starts_with("wip")).unwrap();
        assert_eq!(titles[wip_at + 1], "feat(ui): popup frame with pane tabs");
        assert_eq!(
            titles[titles.len() - 2..],
            ["local · chaoshub · spike", "spike: a faster sync"]
        );
        assert!(
            !titles.contains(&"(no description)"),
            "the empty working copy is hidden"
        );
    }

    #[test]
    fn local_rows_follow_the_narrow_set_and_the_filter() {
        let state = with_local();
        let areas = layout(SIZE);
        let cx = KeyCx {
            data: &state.data,
            target: None,
            areas: &areas,
            keys: &crate::app::keymap::DEFAULT,
        };
        let narrow = Size {
            width: 70,
            height: 20,
        };
        let model = Mine.model(&cx, &Mine.init_state(), narrow);
        for row in &model.rows {
            if !matches!(row.item, RowItem::StackHeader { .. }) {
                assert_eq!(row.cells.len(), model.columns.len());
            }
        }
        let mut ts = Mine.init_state();
        ts.filter = "spike".into();
        Mine.rescore(&cx, &mut ts);
        let titles: Vec<String> = Mine
            .model(&cx, &ts, SIZE)
            .rows
            .iter()
            .map(|r| r.cells[0].text.clone())
            .collect();
        assert_eq!(titles, ["local · chaoshub · spike", "spike: a faster sync"]);
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
        assert_eq!(frame.tab_bar.titles, vec!["Mine", "Team"]);
        assert_eq!(frame.list.rows.len(), 5);
    }
}
