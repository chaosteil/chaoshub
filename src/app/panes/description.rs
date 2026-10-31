//! The description pane of the pull request popup: the body of the pull
//! request as markdown. It is the first pane, so the popup opens on it.

use crate::app::models::{CellStyle, PaneModel};
use crate::app::panes::{Pane, PaneState};
use crate::app::tabs::cells::cell;
use crate::app::tabs::{Handled, KeyCx};
use crate::bus::{KeyChord, Size, Target};

/// The description pane.
pub(crate) struct DescriptionPane;

impl Pane for DescriptionPane {
    fn id(&self) -> &'static str {
        "description"
    }

    fn title(&self) -> &'static str {
        "Description"
    }

    fn key(&self) -> char {
        '1'
    }

    fn applies(&self, target: &Target) -> bool {
        matches!(target, Target::Pr { .. })
    }

    fn init_state(&self) -> PaneState {
        PaneState::default()
    }

    fn model(&self, cx: &KeyCx<'_>, _ps: &PaneState, _size: Size) -> PaneModel {
        let body = match cx.target {
            Some(Target::Pr { pr, .. }) => pr.head.value.body.replace("\r\n", "\n"),
            None => String::new(),
        };
        let body = body.trim();
        let line = if body.is_empty() {
            vec![cell("(no description)", CellStyle::Dim)]
        } else {
            vec![cell(body, CellStyle::Markdown)]
        };
        PaneModel::Lines {
            lines: vec![line],
            wrap: true,
        }
    }

    fn handle_key(&self, _key: KeyChord, _cx: &KeyCx<'_>, _ps: &mut PaneState) -> Option<Handled> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::layout::layout;
    use crate::app::models::Cell;
    use crate::app::state::AppState;
    use crate::app::target::pr_target;
    use crate::app::testdata;

    /// The lines of the pane for a pull request with `body`.
    fn lines_for(body: &str) -> Vec<Vec<Cell>> {
        let mut state = AppState::new(Size {
            width: 100,
            height: 30,
        });
        let mut pr = testdata::pr("chaoshub", 7, "Seven");
        pr.head.value.body = body.into();
        state.data.prs.insert(pr.key.clone(), pr.clone());
        let target = pr_target(&state.data, &pr.key).unwrap();
        assert!(DescriptionPane.applies(&target));
        let areas = layout(state.data.size);
        let cx = KeyCx {
            data: &state.data,
            target: Some(&target),
            areas: &areas,
            keys: &crate::app::keymap::DEFAULT,
        };
        let PaneModel::Lines { lines, wrap } = DescriptionPane.model(
            &cx,
            &PaneState::default(),
            Size {
                width: 80,
                height: 20,
            },
        ) else {
            panic!("not lines");
        };
        assert!(wrap);
        lines
    }

    #[test]
    fn the_body_is_one_markdown_cell() {
        assert_eq!(
            lines_for("\r\n## Why\r\n\r\nThe body.\r\n\r\n"),
            [vec![cell("## Why\n\nThe body.", CellStyle::Markdown)]]
        );
    }

    #[test]
    fn a_blank_body_shows_no_description() {
        for body in ["", "  \r\n\n "] {
            assert_eq!(
                lines_for(body),
                [vec![cell("(no description)", CellStyle::Dim)]],
                "{body:?}"
            );
        }
    }
}
