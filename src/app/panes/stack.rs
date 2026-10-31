//! The stack pane of the pull request popup: the entries of the stack by
//! position, the current pull request highlighted.

use crate::app::models::{Cell, CellStyle, PaneModel};
use crate::app::panes::{Pane, PaneState};
use crate::app::tabs::cells::{self, cell};
use crate::app::tabs::{Handled, KeyCx};
use crate::bus::{KeyChord, Size, Target, TargetKey};

/// The stack pane.
pub(crate) struct StackPane;

impl Pane for StackPane {
    fn id(&self) -> &'static str {
        "stack"
    }

    fn title(&self) -> &'static str {
        "Stack"
    }

    fn key(&self) -> char {
        '4'
    }

    fn applies(&self, target: &Target) -> bool {
        matches!(target, Target::Pr { pr, .. } if pr.head.value.stack.is_some())
    }

    fn init_state(&self) -> PaneState {
        PaneState::default()
    }

    fn model(&self, cx: &KeyCx<'_>, _ps: &PaneState, _size: Size) -> PaneModel {
        let mut lines: Vec<Vec<Cell>> = Vec::new();
        let Some(Target::Pr { pr, stack, .. }) = cx.target else {
            return PaneModel::Lines { lines, wrap: false };
        };
        let Some(info) = &pr.head.value.stack else {
            return PaneModel::Lines { lines, wrap: false };
        };
        let noun = if info.size == 1 { "PR" } else { "PRs" };
        lines.push(vec![cell(
            format!(
                "stack #{} -> {} · {} {noun} · position 1 merges first",
                info.number, info.base_ref, info.size
            ),
            CellStyle::Dim,
        )]);
        lines.push(Vec::new());
        for position in 1..=info.size.max(info.position) {
            let entry = stack.iter().find(|entry| {
                entry
                    .head
                    .value
                    .stack
                    .as_ref()
                    .is_some_and(|s| s.position == position)
            });
            let Some(entry) = entry else {
                lines.push(vec![
                    cell(format!("  {position} "), CellStyle::Dim),
                    cell("not loaded", CellStyle::Dim),
                ]);
                continue;
            };
            let current = entry.key == pr.key;
            let (marker, title_style) = if current {
                ("› ", CellStyle::Primary)
            } else {
                ("  ", CellStyle::Normal)
            };
            lines.push(vec![
                cell(format!("{marker}{position} "), CellStyle::Dim),
                cell(format!("#{} ", entry.key.number), CellStyle::Primary),
                cell(entry.head.value.title.clone(), title_style),
                cell("  ", CellStyle::Dim),
                cells::ci(entry.ci_state.value),
                cell(" ", CellStyle::Dim),
                cells::review(entry.head.value.review_decision),
                cell(" ", CellStyle::Dim),
                cells::flags(entry, cx.data.pane_open(&TargetKey::Pr(entry.key.clone()))),
            ]);
        }
        PaneModel::Lines { lines, wrap: false }
    }

    fn handle_key(&self, _key: KeyChord, _cx: &KeyCx<'_>, _ps: &mut PaneState) -> Option<Handled> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::layout::layout;
    use crate::app::state::AppState;
    use crate::app::target::pr_target;
    use crate::app::testdata;

    fn lines_for(state: &AppState, number: u64) -> Option<Vec<String>> {
        let target = pr_target(&state.data, &testdata::pr_key("chaoshub", number))?;
        if !StackPane.applies(&target) {
            return None;
        }
        let areas = layout(state.data.size);
        let cx = KeyCx {
            data: &state.data,
            target: Some(&target),
            areas: &areas,
            keys: &crate::app::keymap::DEFAULT,
        };
        let PaneModel::Lines { lines, .. } = StackPane.model(
            &cx,
            &PaneState::default(),
            Size {
                width: 80,
                height: 20,
            },
        ) else {
            panic!("not lines");
        };
        Some(
            lines
                .iter()
                .map(|line| line.iter().map(|c| c.text.as_str()).collect())
                .collect(),
        )
    }

    #[test]
    fn entries_by_position_with_the_current_one_marked() {
        let mut state = AppState::new(Size {
            width: 100,
            height: 30,
        });
        state.load(testdata::mine_snapshot(), Vec::new());
        assert_eq!(
            lines_for(&state, 102).unwrap(),
            [
                "stack #12 -> main · 3 PRs · position 1 merges first",
                "",
                "  1 #101 feat(app): Pane trait, registry and overlay stack  ✗ ✗ C",
                "› 2 #102 feat(app): checks pane  ✓ ✓ ",
                "  3 #103 feat(ui): popup frame with pane tabs  ● ○ D",
            ]
        );
        state.data.prs.remove(&testdata::pr_key("chaoshub", 101));
        assert_eq!(lines_for(&state, 102).unwrap()[2], "  1 not loaded");
        assert_eq!(
            lines_for(&state, 99),
            None,
            "a single pull request has no stack pane"
        );
    }
}
