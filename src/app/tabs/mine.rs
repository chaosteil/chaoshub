//! The Mine tab: the pull requests of the user.
//!
//! For now the tab shows one row per pull request with the title only.
//! The stack tree, the jj columns and the keys come in later phases.

use crate::app::models::{
    Align, Cell, CellStyle, Column, ColumnWidth, ListModel, Row, RowItem, RowStyle,
};
use crate::app::tabs::{Handled, KeyCx, Tab, TabState};
use crate::bus::{Interest, KeyChord, PrRef, Scope, Size};
use crate::domain::pr::PullRequest;

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
        let rows: Vec<Row> = cx.data.prs.values().map(row).collect();
        ListModel {
            columns: vec![Column {
                title: "Title".into(),
                width: ColumnWidth::Fill {
                    min: TITLE_MIN_WIDTH,
                },
                align: Align::Left,
            }],
            cursor: ts.cursor.min(rows.len().saturating_sub(1)),
            scroll: ts.scroll,
            rows,
        }
    }

    fn interest(&self, cx: &KeyCx<'_>, ts: &TabState) -> Interest {
        let prs = || cx.data.prs.values();
        Interest {
            tab: self.id(),
            scope: self.scope(),
            wants_local: self.wants_local(),
            selected: prs().nth(ts.cursor).map(pr_ref),
            visible: prs()
                .skip(ts.scroll)
                .take(usize::from(cx.areas.list.height))
                .map(pr_ref)
                .collect(),
        }
    }

    fn handle_key(&self, _key: KeyChord, _cx: &KeyCx<'_>, _ts: &mut TabState) -> Option<Handled> {
        None
    }

    fn help(&self) -> Vec<(KeyChord, &'static str)> {
        Vec::new()
    }
}

/// The row of a pull request: the title only.
fn row(pr: &PullRequest) -> Row {
    Row {
        item: RowItem::Pr(pr_ref(pr)),
        cells: vec![Cell {
            text: pr.head.value.title.clone(),
            style: CellStyle::Normal,
        }],
        depth: 0,
        glyph: None,
        style: RowStyle::Normal,
    }
}

/// The fields of a pull request that the workers need.
fn pr_ref(pr: &PullRequest) -> PrRef {
    PrRef {
        key: pr.key.clone(),
        node_id: pr.node_id.clone(),
        head_oid: pr.head.value.head_oid.clone(),
        base_ref: pr.head.value.base_ref.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::layout::layout;
    use crate::app::state::AppState;
    use crate::domain::keys::{PrKey, RepoKey};
    use crate::domain::observed::Observed;
    use crate::domain::pr::{CiState, MergeStateStatus, Mergeable, PrHead, PrState};
    use crate::domain::repo::RepoSettings;
    use crate::domain::test_time;

    const SIZE: Size = Size {
        width: 100,
        height: 30,
    };

    fn observed<T>(value: T) -> Observed<T> {
        Observed {
            value,
            observed_at: test_time(),
        }
    }

    fn pr(repo: &str, number: u64, title: &str) -> PullRequest {
        PullRequest {
            key: PrKey {
                repo: RepoKey {
                    owner: "chaosteil".into(),
                    name: repo.into(),
                },
                number,
            },
            node_id: format!("PR_{repo}_{number}"),
            head: observed(PrHead {
                title: title.into(),
                body: String::new(),
                author: "chaosteil".into(),
                state: PrState::Open,
                is_draft: false,
                head_ref: format!("chaosteil/{number}"),
                head_oid: format!("oid{number}"),
                base_ref: "main".into(),
                review_decision: None,
                review_requested: false,
                reviewers: Vec::new(),
                stack: None,
                auto_merge: false,
                in_merge_queue: false,
                mergeable: Mergeable::Unknown,
                merge_state: MergeStateStatus::Unknown,
                url: format!("https://github.com/chaosteil/{repo}/pull/{number}"),
                updated_at: test_time(),
                repo_settings: RepoSettings {
                    squash: true,
                    merge: true,
                    rebase: true,
                    default_branch: None,
                },
            }),
            ci_state: observed(CiState::None),
            checks: observed(Vec::new()),
        }
    }

    /// A state with three pull requests, seeded out of order.
    fn seeded() -> (AppState, [PullRequest; 3]) {
        let prs = [
            pr("chaoshub", 7, "Add the event loop"),
            pr("chaoshub", 12, "Add the mine tab"),
            pr("codeview", 3, "Fix the export"),
        ];
        let mut state = AppState::new(SIZE);
        for pr in [&prs[2], &prs[0], &prs[1]] {
            state.data.prs.insert(pr.key.clone(), pr.clone());
        }
        (state, prs)
    }

    fn title_row(pr: &PullRequest) -> Row {
        Row {
            item: RowItem::Pr(pr_ref(pr)),
            cells: vec![Cell {
                text: pr.head.value.title.clone(),
                style: CellStyle::Normal,
            }],
            depth: 0,
            glyph: None,
            style: RowStyle::Normal,
        }
    }

    #[test]
    fn one_title_row_per_pull_request() {
        let (state, prs) = seeded();
        let areas = layout(SIZE);
        let cx = KeyCx {
            data: &state.data,
            target: None,
            areas: &areas,
        };
        let ts = TabState {
            cursor: 9,
            ..TabState::default()
        };
        assert_eq!(
            Mine.model(&cx, &ts, SIZE),
            ListModel {
                columns: vec![Column {
                    title: "Title".into(),
                    width: ColumnWidth::Fill { min: 28 },
                    align: Align::Left,
                }],
                rows: prs.iter().map(title_row).collect(),
                cursor: 2,
                scroll: 0,
            }
        );
    }

    #[test]
    fn interest_names_the_selected_and_visible_rows() {
        let (state, prs) = seeded();
        let areas = layout(SIZE);
        let cx = KeyCx {
            data: &state.data,
            target: None,
            areas: &areas,
        };
        let ts = TabState {
            cursor: 1,
            scroll: 1,
            ..TabState::default()
        };
        assert_eq!(
            Mine.interest(&cx, &ts),
            Interest {
                tab: "mine",
                scope: Some(Scope::Mine),
                wants_local: true,
                selected: Some(pr_ref(&prs[1])),
                visible: vec![pr_ref(&prs[1]), pr_ref(&prs[2])],
            }
        );
    }

    #[test]
    fn the_frame_shows_the_mine_tab() {
        let (mut state, prs) = seeded();
        let frame = crate::app::frame::frame_model(&mut state);
        assert_eq!(frame.tab_bar.titles, vec!["Mine"]);
        assert_eq!(
            frame.list.rows,
            prs.iter().map(title_row).collect::<Vec<_>>()
        );
    }
}
