//! The Team tab: the open pull requests of colleagues.
//!
//! By default it shows the ready pull requests (`domain::team::is_ready`).
//! The `toggle_all` key (`t`) shows every open pull request. Pull requests
//! that request the review of the user come first, marked with `★`.

use crate::app::age::age;
use crate::app::filter::{Filter, ItemKey, pr_candidate};
use crate::app::models::{
    Align, Cell, CellStyle, Column, ColumnWidth, ListModel, Row, RowItem, RowStyle,
};
use crate::app::nav::place;
use crate::app::tabs::cells::{self, cell};
use crate::app::tabs::{Handled, KeyCx, Tab, TabState, list_interest, pr_ref};
use crate::bus::{Interest, KeyChord, Scope, Size};
use crate::domain::pr::{PrState, PullRequest};
use crate::domain::team::{is_ready, team_order};

/// The minimum width of the title column.
const TITLE_MIN_WIDTH: u16 = 20;

/// The Team tab.
pub(crate) struct Team;

/// The own state of the Team tab.
#[derive(Debug, Default)]
pub(crate) struct TeamState {
    pub filter: Filter,
    /// True when the tab shows every open pull request, not only the
    /// ready ones.
    pub all_open: bool,
}

impl Tab for Team {
    fn id(&self) -> &'static str {
        "team"
    }

    fn title(&self) -> &'static str {
        "Team"
    }

    fn scope(&self) -> Option<Scope> {
        Some(Scope::Team)
    }

    fn wants_local(&self) -> bool {
        false
    }

    fn init_state(&self) -> TabState {
        TabState {
            ext: Box::new(TeamState::default()),
            ..TabState::default()
        }
    }

    fn model(&self, cx: &KeyCx<'_>, ts: &TabState, size: Size) -> ListModel {
        let ext = ts.ext.downcast_ref::<TeamState>();
        let all_open = ext.is_some_and(|ext| ext.all_open);
        let mut prs = team_prs(cx, all_open);
        if let Some(ext) = ext {
            prs.retain(|pr| ext.filter.keeps(&ItemKey::Pr(pr.key.clone())));
        }
        let mut rows = vec![mode_row(all_open, prs.len())];
        rows.extend(prs.into_iter().map(|pr| pr_row(pr, cx)));
        let (cursor, scroll) = place(
            &rows,
            ts.selected.as_ref(),
            ts.cursor,
            ts.scroll,
            usize::from(size.height),
        );
        ListModel {
            columns: columns(),
            rows,
            cursor,
            scroll,
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
        let Some(ext) = ts.ext.downcast_mut::<TeamState>() else {
            return;
        };
        let candidates: Vec<_> = team_prs(cx, true).into_iter().map(pr_candidate).collect();
        ext.filter.rescore(&ts.filter, cx.data.revision, candidates);
    }

    fn handle_key(&self, key: KeyChord, cx: &KeyCx<'_>, ts: &mut TabState) -> Option<Handled> {
        if !cx.keys.matches("toggle_all", key) {
            return None;
        }
        let ext = ts.ext.downcast_mut::<TeamState>()?;
        ext.all_open = !ext.all_open;
        Some(Handled {
            changed: true,
            ..Handled::default()
        })
    }

    fn help(&self) -> Vec<(KeyChord, &'static str)> {
        Vec::new()
    }
}

/// The columns: requested marker, title, number, repo, author, updated,
/// CI, review and flags.
fn columns() -> Vec<Column> {
    let column = |title: &str, width, align| Column {
        title: title.into(),
        width,
        align,
    };
    vec![
        column("", ColumnWidth::Fixed(2), Align::Left),
        column(
            "title",
            ColumnWidth::Fill {
                min: TITLE_MIN_WIDTH,
            },
            Align::Left,
        ),
        column("#", ColumnWidth::Fixed(6), Align::Right),
        column("repo", ColumnWidth::Fixed(12), Align::Left),
        column("author", ColumnWidth::Fixed(12), Align::Left),
        column("updated", ColumnWidth::Fixed(6), Align::Right),
        column("ci", ColumnWidth::Fixed(3), Align::Center),
        column("rv", ColumnWidth::Fixed(3), Align::Center),
        column("flags", ColumnWidth::Fixed(4), Align::Left),
    ]
}

/// The open pull requests of the Team scope in team order, only the
/// ready ones unless `all_open`.
fn team_prs<'a>(cx: &KeyCx<'a>, all_open: bool) -> Vec<&'a PullRequest> {
    let data = cx.data;
    let mut prs: Vec<&PullRequest> = data
        .prs
        .values()
        .filter(|pr| data.scopes.get(&pr.key) == Some(&Scope::Team))
        .filter(|pr| pr.head.value.state == PrState::Open)
        .filter(|pr| all_open || is_ready(pr))
        .collect();
    prs.sort_by(|a, b| team_order(a, b));
    prs
}

/// The header row that names the mode and the key that changes it.
fn mode_row(all_open: bool, count: usize) -> Row {
    let text = if all_open {
        format!("all open · {count} · t shows ready only")
    } else {
        format!("ready for review · {count} · t shows all open")
    };
    Row {
        item: RowItem::StackHeader {
            stack_id: "team".into(),
        },
        cells: vec![cell(text, CellStyle::Dim)],
        depth: 0,
        glyph: None,
        style: RowStyle::Header,
    }
}

/// The row of one pull request.
fn pr_row(pr: &PullRequest, cx: &KeyCx<'_>) -> Row {
    let head = &pr.head.value;
    let star = if head.review_requested { "★" } else { "" };
    let cells: Vec<Cell> = vec![
        cell(star, CellStyle::Warn),
        cell(head.title.clone(), CellStyle::Normal),
        cell(format!("#{}", pr.key.number), CellStyle::Dim),
        cell(pr.key.repo.name.clone(), CellStyle::Dim),
        cell(head.author.clone(), CellStyle::Info),
        cell(age(head.updated_at, cx.data.now), CellStyle::Dim),
        cells::ci(pr.ci_state.value),
        cells::review(head.review_decision),
        cells::flags(pr),
    ];
    Row {
        item: RowItem::Pr(pr_ref(pr)),
        cells,
        depth: 0,
        glyph: None,
        style: RowStyle::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::keymap::Keymap;
    use crate::app::layout::layout;
    use crate::app::state::AppState;
    use crate::app::testdata;
    use crate::bus::{KeyCode, KeyModifiers};

    const SIZE: Size = Size {
        width: 100,
        height: 30,
    };

    fn seeded() -> AppState {
        let mut state = AppState::new(SIZE);
        state.data.now = crate::domain::test_time();
        state.load(testdata::team_snapshot(), Vec::new());
        state
    }

    fn numbers(model: &ListModel) -> Vec<u64> {
        model
            .rows
            .iter()
            .filter_map(|row| match &row.item {
                RowItem::Pr(pr) => Some(pr.key.number),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn ready_prs_with_requests_first_and_t_shows_all() {
        let state = seeded();
        let areas = layout(SIZE);
        let keys = Keymap::default();
        let cx = KeyCx {
            data: &state.data,
            target: None,
            areas: &areas,
            keys: &keys,
        };
        let mut ts = Team.init_state();
        let model = Team.model(&cx, &ts, SIZE);
        assert_eq!(numbers(&model), [88, 87, 86]);
        assert_eq!(
            model.rows[0].cells[0].text,
            "ready for review · 3 · t shows all open"
        );
        assert_eq!(model.cursor, 1, "the cursor skips the mode row");
        let t = KeyChord {
            code: KeyCode::Char('t'),
            mods: KeyModifiers::NONE,
        };
        assert_eq!(
            Team.handle_key(t, &cx, &mut ts),
            Some(Handled {
                changed: true,
                ..Handled::default()
            })
        );
        let model = Team.model(&cx, &ts, SIZE);
        assert_eq!(numbers(&model), [85, 88, 87, 84, 86]);
        let x = KeyChord {
            code: KeyCode::Char('x'),
            mods: KeyModifiers::NONE,
        };
        assert_eq!(Team.handle_key(x, &cx, &mut ts), None);
    }

    #[test]
    fn a_row_has_every_column() {
        let state = seeded();
        let areas = layout(SIZE);
        let cx = KeyCx {
            data: &state.data,
            target: None,
            areas: &areas,
            keys: &crate::app::keymap::DEFAULT,
        };
        let model = Team.model(&cx, &Team.init_state(), SIZE);
        assert_eq!(model.columns.len(), 9);
        let first = &model.rows[1];
        assert_eq!(
            first.cells,
            vec![
                cell("★", CellStyle::Warn),
                cell("docs(readme): keymap table", CellStyle::Normal),
                cell("#88", CellStyle::Dim),
                cell("chaoshub", CellStyle::Dim),
                cell("alice", CellStyle::Info),
                cell("2h", CellStyle::Dim),
                cell("✓", CellStyle::Success),
                cell("○", CellStyle::Dim),
                cell("", CellStyle::Warn),
            ]
        );
        let interest = Team.interest(&cx, &Team.init_state());
        assert_eq!(interest.scope, Some(Scope::Team));
        assert!(!interest.wants_local);
        assert_eq!(interest.selected.map(|pr| pr.key.number), Some(88));
    }
}
