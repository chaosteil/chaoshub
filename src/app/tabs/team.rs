//! The Review tab and the Team tab: the open pull requests of colleagues.
//!
//! The Review tab shows the pull requests that request the review of the
//! user. The Team tab shows the other pull requests of colleagues. By
//! default, each tab shows the ready pull requests
//! (`domain::team::is_ready`). The `toggle_all` key (`t`) shows every open
//! pull request.

use crate::app::age::age;
use crate::app::filter::{Filter, ItemKey, pr_candidate};
use crate::app::models::{
    Align, Cell, CellStyle, Column, ColumnWidth, ListModel, Row, RowItem, RowStyle,
};
use crate::app::nav::place;
use crate::app::tabs::cells::{self, cell};
use crate::app::tabs::{Handled, KeyCx, Tab, TabState, list_interest, pr_ref};
use crate::bus::{Interest, KeyChord, Scope, Size, TargetKey};
use crate::domain::pr::{PrState, PullRequest};
use crate::domain::team::{is_ready, team_order};

/// The minimum width of the title column.
const TITLE_MIN_WIDTH: u16 = 20;

/// A tab of the open pull requests of colleagues.
pub(crate) enum Team {
    /// The pull requests that request the review of the user.
    Review,
    /// The other pull requests of colleagues.
    Rest,
}

/// The own state of the Review tab and of the Team tab.
#[derive(Debug, Default)]
pub(crate) struct TeamState {
    pub filter: Filter,
    /// True when the tab shows every open pull request, not only the
    /// ready ones.
    pub all_open: bool,
}

impl Tab for Team {
    fn id(&self) -> &'static str {
        match self {
            Self::Review => "review",
            Self::Rest => "team",
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Self::Review => "Review",
            Self::Rest => "Team",
        }
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
        let mut prs = self.team_prs(cx, all_open);
        if let Some(author) = &ts.author {
            prs.retain(|pr| pr.head.value.author == *author);
        }
        if let Some(ext) = ext {
            prs.retain(|pr| ext.filter.keeps(&ItemKey::Pr(pr.key.clone())));
        }
        let mut rows = vec![mode_row(
            self.id(),
            all_open,
            ts.author.as_deref(),
            prs.len(),
        )];
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
        let candidates: Vec<_> = self
            .team_prs(cx, true)
            .into_iter()
            .map(pr_candidate)
            .collect();
        ext.filter.rescore(&ts.filter, cx.data.revision, candidates);
    }

    fn authors(&self, cx: &KeyCx<'_>) -> Vec<String> {
        let mut authors: Vec<String> = self
            .team_prs(cx, true)
            .into_iter()
            .map(|pr| pr.head.value.author.clone())
            .collect();
        authors.sort();
        authors.dedup();
        authors
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

/// The columns: title, number, repo, author, updated, checks, reviews
/// and flags.
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
        column("repo", ColumnWidth::Fixed(12), Align::Left),
        column("author", ColumnWidth::Fixed(12), Align::Left),
        column("upd", ColumnWidth::Fixed(6), Align::Right),
        column("checks", ColumnWidth::Fixed(7), Align::Center),
        column("rev", ColumnWidth::Fixed(5), Align::Center),
        column("flg", ColumnWidth::Fixed(4), Align::Left),
    ]
}

impl Team {
    /// The open pull requests of the Team scope that belong to this tab,
    /// in team order. Only the ready ones unless `all_open`.
    fn team_prs<'a>(&self, cx: &KeyCx<'a>, all_open: bool) -> Vec<&'a PullRequest> {
        let requested = matches!(self, Self::Review);
        let data = cx.data;
        let mut prs: Vec<&PullRequest> = data
            .prs
            .values()
            .filter(|pr| data.scopes.get(&pr.key) == Some(&Scope::Team))
            .filter(|pr| pr.head.value.state == PrState::Open)
            .filter(|pr| pr.head.value.review_requested == requested)
            .filter(|pr| all_open || is_ready(pr))
            .collect();
        prs.sort_by(|a, b| team_order(a, b));
        prs
    }
}

/// The header row that names the mode, the author filter and the key
/// that changes the mode.
fn mode_row(stack_id: &str, all_open: bool, author: Option<&str>, count: usize) -> Row {
    let by = author.map(|a| format!(" · by {a}")).unwrap_or_default();
    let text = if all_open {
        format!("all open · {count}{by} · t shows ready only")
    } else {
        format!("ready for review · {count}{by} · t shows all open")
    };
    Row {
        item: RowItem::StackHeader {
            stack_id: stack_id.into(),
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
    let cells: Vec<Cell> = vec![
        cell(head.title.clone(), CellStyle::Normal),
        cell(format!("#{}", pr.key.number), CellStyle::Dim),
        cell(pr.key.repo.name.clone(), CellStyle::Dim),
        cell(head.author.clone(), CellStyle::Info),
        cell(age(head.updated_at, cx.data.now), CellStyle::Dim),
        cells::checks(pr),
        cells::reviews(head),
        cells::flags(pr, cx.data.pane_open(&TargetKey::Pr(pr.key.clone()))),
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
    fn ready_prs_and_t_shows_all() {
        let state = seeded();
        let areas = layout(SIZE);
        let keys = Keymap::default();
        let cx = KeyCx {
            data: &state.data,
            target: None,
            areas: &areas,
            keys: &keys,
        };
        let t = KeyChord {
            code: KeyCode::Char('t'),
            mods: KeyModifiers::NONE,
        };

        let mut ts = Team::Rest.init_state();
        let model = Team::Rest.model(&cx, &ts, SIZE);
        assert_eq!(numbers(&model), [87, 86]);
        assert_eq!(
            model.rows[0].cells[0].text,
            "ready for review · 2 · t shows all open"
        );
        assert_eq!(model.cursor, 1, "the cursor skips the mode row");
        assert_eq!(
            Team::Rest.handle_key(t, &cx, &mut ts),
            Some(Handled {
                changed: true,
                ..Handled::default()
            })
        );
        let model = Team::Rest.model(&cx, &ts, SIZE);
        assert_eq!(numbers(&model), [87, 84, 86]);
        let x = KeyChord {
            code: KeyCode::Char('x'),
            mods: KeyModifiers::NONE,
        };
        assert_eq!(Team::Rest.handle_key(x, &cx, &mut ts), None);

        let mut ts = Team::Review.init_state();
        let model = Team::Review.model(&cx, &ts, SIZE);
        assert_eq!(numbers(&model), [88]);
        assert_eq!(
            model.rows[0].cells[0].text,
            "ready for review · 1 · t shows all open"
        );
        assert_eq!(
            Team::Review.handle_key(t, &cx, &mut ts),
            Some(Handled {
                changed: true,
                ..Handled::default()
            })
        );
        let model = Team::Review.model(&cx, &ts, SIZE);
        assert_eq!(numbers(&model), [85, 88]);
    }

    #[test]
    fn authors_and_the_author_filter() {
        let state = seeded();
        let areas = layout(SIZE);
        let cx = KeyCx {
            data: &state.data,
            target: None,
            areas: &areas,
            keys: &crate::app::keymap::DEFAULT,
        };
        assert_eq!(Team::Rest.authors(&cx), ["bob", "dave"]);
        assert_eq!(Team::Review.authors(&cx), ["alice", "carol"]);
        let ts = TabState {
            author: Some("bob".into()),
            ..Team::Rest.init_state()
        };
        let model = Team::Rest.model(&cx, &ts, SIZE);
        assert_eq!(numbers(&model), [87, 86]);
        assert_eq!(
            model.rows[0].cells[0].text,
            "ready for review · 2 · by bob · t shows all open"
        );
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
        let model = Team::Review.model(&cx, &Team::Review.init_state(), SIZE);
        assert_eq!(model.columns.len(), 8);
        let first = &model.rows[1];
        assert_eq!(
            first.cells,
            vec![
                cell("docs(readme): keymap table", CellStyle::Normal),
                cell("#88", CellStyle::Dim),
                cell("chaoshub", CellStyle::Dim),
                cell("alice", CellStyle::Info),
                cell("2h", CellStyle::Dim),
                cell("·", CellStyle::Dim),
                cell("○", CellStyle::Dim),
                cell("", CellStyle::Warn),
            ]
        );
        let interest = Team::Review.interest(&cx, &Team::Review.init_state());
        assert_eq!(interest.scope, Some(Scope::Team));
        assert!(!interest.wants_local);
        assert_eq!(interest.selected.map(|pr| pr.key.number), Some(88));
        let rest = Team::Rest.interest(&cx, &Team::Rest.init_state());
        assert_eq!(
            (
                interest.tab,
                rest.tab,
                rest.selected.map(|pr| pr.key.number)
            ),
            ("review", "team", Some(87))
        );
    }
}
