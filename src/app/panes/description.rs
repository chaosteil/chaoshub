//! The description pane of the pull request popup: the body of the pull
//! request as markdown. It is the first pane, so the popup opens on it.

use crate::app::models::{Cell, CellStyle, PaneModel};
use crate::app::panes::{Pane, PaneState};
use crate::app::tabs::cells::cell;
use crate::app::tabs::{Handled, KeyCx};
use crate::bus::{KeyChord, Size, Target};
use crate::domain::pr::PrHead;

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
        let Some(Target::Pr { pr, .. }) = cx.target else {
            return PaneModel::Lines {
                lines: Vec::new(),
                wrap: true,
            };
        };
        let head = &pr.head.value;
        let body = head.body.replace("\r\n", "\n");
        let body = body.trim();
        let line = if body.is_empty() {
            vec![cell("(no description)", CellStyle::Dim)]
        } else {
            vec![cell(body, CellStyle::Markdown)]
        };
        PaneModel::Lines {
            lines: vec![summary(head), Vec::new(), line],
            wrap: true,
        }
    }

    fn handle_key(&self, _key: KeyChord, _cx: &KeyCx<'_>, _ps: &mut PaneState) -> Option<Handled> {
        None
    }
}

/// The size of the change: `12 files changed · +340 −56`.
fn summary(head: &PrHead) -> Vec<Cell> {
    let files = if head.changed_files == 1 {
        "1 file changed".to_owned()
    } else {
        format!("{} files changed", head.changed_files)
    };
    vec![
        cell(files, CellStyle::Primary),
        cell(" · ", CellStyle::Dim),
        cell(format!("+{}", head.additions), CellStyle::Success),
        cell(format!(" −{}", head.deletions), CellStyle::Danger),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::layout::layout;
    use crate::app::state::AppState;
    use crate::app::target::pr_target;
    use crate::app::testdata;

    /// The lines of the pane for a pull request with `body`.
    fn lines_for(body: &str) -> Vec<Vec<Cell>> {
        lines_with(body, |_| {})
    }

    /// The lines of the pane for a pull request with `body`, after `edit`
    /// changes its head.
    fn lines_with(body: &str, edit: impl FnOnce(&mut PrHead)) -> Vec<Vec<Cell>> {
        let mut state = AppState::new(Size {
            width: 100,
            height: 30,
        });
        let mut pr = testdata::pr("chaoshub", 7, "Seven");
        pr.head.value.body = body.into();
        edit(&mut pr.head.value);
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

    /// The summary line of the test pull request: 3 files, +42 −7.
    fn test_summary() -> Vec<Cell> {
        vec![
            cell("3 files changed", CellStyle::Primary),
            cell(" · ", CellStyle::Dim),
            cell("+42", CellStyle::Success),
            cell(" −7", CellStyle::Danger),
        ]
    }

    #[test]
    fn the_body_is_one_markdown_cell() {
        assert_eq!(
            lines_for("\r\n## Why\r\n\r\nThe body.\r\n\r\n"),
            [
                test_summary(),
                Vec::new(),
                vec![cell("## Why\n\nThe body.", CellStyle::Markdown)]
            ]
        );
    }

    #[test]
    fn a_blank_body_shows_no_description() {
        for body in ["", "  \r\n\n "] {
            assert_eq!(
                lines_for(body),
                [
                    test_summary(),
                    Vec::new(),
                    vec![cell("(no description)", CellStyle::Dim)]
                ],
                "{body:?}"
            );
        }
    }

    #[test]
    fn the_summary_line_counts_the_files_and_the_lines() {
        let one = lines_with("", |head| {
            head.changed_files = 1;
            head.additions = 5;
            head.deletions = 0;
        });
        assert_eq!(
            one[0],
            [
                cell("1 file changed", CellStyle::Primary),
                cell(" · ", CellStyle::Dim),
                cell("+5", CellStyle::Success),
                cell(" −0", CellStyle::Danger),
            ]
        );
        let many = lines_with("", |head| {
            head.changed_files = 12;
            head.additions = 340;
            head.deletions = 56;
        });
        assert_eq!(
            many[0],
            [
                cell("12 files changed", CellStyle::Primary),
                cell(" · ", CellStyle::Dim),
                cell("+340", CellStyle::Success),
                cell(" −56", CellStyle::Danger),
            ]
        );
    }
}
