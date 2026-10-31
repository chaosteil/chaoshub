//! The `Pane` trait and the pane registry.
//!
//! A pane is one sub-view of the pull request popup. The popup shows the
//! panes whose `applies` is true, in registry order. A new pane is one
//! module plus one line in `PANES`.

mod checks;
pub(crate) mod conversation;
mod description;
mod stack;

use std::any::Any;
use std::sync::LazyLock;

use crate::app::models::{Cell, CellStyle, PaneModel};
use crate::app::tabs::{Handled, KeyCx};
use crate::bus::{KeyChord, Size, Target};

/// The state of one pane, made by `Pane::init_state`.
#[derive(Debug)]
pub(crate) struct PaneState {
    pub cursor: usize,
    pub scroll: usize,
    /// The own state of the pane, for example the expanded threads.
    pub ext: Box<dyn Any + Send + Sync>,
}

impl Default for PaneState {
    /// The cursor and the scroll at the top, no own state.
    fn default() -> Self {
        Self {
            cursor: 0,
            scroll: 0,
            ext: Box::new(()),
        }
    }
}

/// One sub-view of the pull request popup.
pub(crate) trait Pane: Send + Sync {
    /// The id, for example `checks`.
    #[allow(
        dead_code,
        reason = "the registry test reads it, and phase E keys pane actions by it"
    )]
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;
    /// The key that selects the pane in the popup, for example `1`.
    fn key(&self) -> char;
    fn applies(&self, target: &Target) -> bool;
    fn init_state(&self) -> PaneState;
    fn model(&self, cx: &KeyCx<'_>, ps: &PaneState, size: Size) -> PaneModel;
    /// `None` when the pane does not handle the key.
    fn handle_key(&self, key: KeyChord, cx: &KeyCx<'_>, ps: &mut PaneState) -> Option<Handled>;
}

/// The pane registry, in popup order.
pub(crate) static PANES: LazyLock<Vec<Box<dyn Pane>>> = LazyLock::new(|| {
    vec![
        Box::new(description::DescriptionPane),
        Box::new(checks::Checks),
        Box::new(conversation::Conversation),
        Box::new(stack::StackPane),
    ]
});

/// The registry indexes of the panes that apply to `target`.
pub(crate) fn applicable(panes: &[Box<dyn Pane>], target: &Target) -> Vec<usize> {
    panes
        .iter()
        .enumerate()
        .filter(|(_, pane)| pane.applies(target))
        .map(|(i, _)| i)
        .collect()
}

/// The number of items that the cursor of a pane moves over: rows,
/// nodes or lines.
pub(crate) fn items(model: &PaneModel) -> usize {
    match model {
        PaneModel::Table { rows, .. } => rows.len(),
        PaneModel::Lines { lines, .. } => lines.len(),
        PaneModel::Tree { nodes } => nodes.len(),
    }
}

/// The flat margin of rows for the block spacing of the markdown
/// renderer, once per markdown cell.
const BLOCK_MARGIN: usize = 2;

/// The blank rows that the markdown renderer adds around a heading: one
/// before and one after.
const HEADING_ROWS: usize = 2;

/// The display rows of the lines of a pane of lines at `width`. A line
/// without a markdown cell takes one row. A line with a markdown cell
/// takes one row or more per source line of the cell text, plus
/// `HEADING_ROWS` per heading line (a line that starts with `#`), plus
/// `BLOCK_MARGIN`. The estimate can be a few rows too high, but never too
/// low, so the scroll can reach the last row.
pub(crate) fn display_rows(lines: &[Vec<Cell>], width: u16) -> usize {
    let width = usize::from(width.max(1));
    lines
        .iter()
        .map(|cells| {
            let Some(markdown) = cells.iter().find(|c| c.style == CellStyle::Markdown) else {
                return 1;
            };
            let rows: usize = markdown
                .text
                .lines()
                .map(|line| {
                    let rows = textwrap::core::display_width(line).div_ceil(width).max(1);
                    if line.trim_start().starts_with('#') {
                        rows + HEADING_ROWS
                    } else {
                        rows
                    }
                })
                .sum();
            rows + BLOCK_MARGIN
        })
        .sum()
}

/// A pane for the tests of the popup.
#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use crate::app::models::{Cell, CellStyle};
    use crate::bus::{Cmd, KeyCode};

    /// Shows `lines` numbered lines. `x` returns `Cmd::OpenUrl` with the
    /// cursor line; it applies only when `only` is `None` or the number of
    /// the pull request.
    pub(crate) struct FakePane {
        pub id: &'static str,
        pub key: char,
        pub lines: usize,
        pub only: Option<u64>,
    }

    impl Pane for FakePane {
        fn id(&self) -> &'static str {
            self.id
        }

        fn title(&self) -> &'static str {
            self.id
        }

        fn key(&self) -> char {
            self.key
        }

        fn applies(&self, target: &Target) -> bool {
            match (target, self.only) {
                (_, None) => true,
                (Target::Pr { pr, .. }, Some(number)) => pr.key.number == number,
            }
        }

        fn init_state(&self) -> PaneState {
            PaneState::default()
        }

        fn model(&self, _cx: &KeyCx<'_>, _ps: &PaneState, _size: Size) -> PaneModel {
            PaneModel::Tree {
                nodes: (0..self.lines)
                    .map(|i| crate::app::models::TreeNode {
                        depth: 0,
                        expanded: None,
                        cells: vec![Cell {
                            text: format!("{} {i}", self.id),
                            style: CellStyle::Normal,
                        }],
                    })
                    .collect(),
            }
        }

        fn handle_key(
            &self,
            key: KeyChord,
            _cx: &KeyCx<'_>,
            ps: &mut PaneState,
        ) -> Option<Handled> {
            (key.code == KeyCode::Char('x')).then(|| Handled {
                cmds: vec![Cmd::OpenUrl(format!("{} {}", self.id, ps.cursor))],
                events: Vec::new(),
                changed: false,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tabs::duplicates;

    #[test]
    fn pane_ids_and_keys_are_unique() {
        assert_eq!(duplicates(PANES.iter().map(|pane| pane.id())), [""; 0]);
        assert_eq!(duplicates(PANES.iter().map(|pane| pane.key())), [' '; 0]);
    }

    #[test]
    fn the_description_is_the_first_pane() {
        let order: Vec<(char, &str)> = PANES.iter().map(|pane| (pane.key(), pane.id())).collect();
        assert_eq!(
            order,
            [
                ('1', "description"),
                ('2', "checks"),
                ('3', "conversation"),
                ('4', "stack"),
            ]
        );
    }

    #[test]
    fn display_rows_cases() {
        use crate::app::tabs::cells::cell;
        let plain = |text: &str| vec![cell(text, CellStyle::Normal)];
        let markdown = |text: &str| vec![cell(text, CellStyle::Markdown)];
        let cases: Vec<(&str, Vec<Vec<Cell>>, u16, usize)> = vec![
            ("no lines", Vec::new(), 10, 0),
            (
                "plain lines",
                vec![plain("a"), plain("b c d e f g h")],
                4,
                2,
            ),
            ("empty line", vec![Vec::new()], 10, 1),
            ("one short line", vec![markdown("abc")], 10, 1 + 2),
            ("exact width", vec![markdown("abcdefghij")], 10, 1 + 2),
            ("one row more", vec![markdown("abcdefghijk")], 10, 2 + 2),
            ("blank source line", vec![markdown("a\n\nb")], 10, 3 + 2),
            ("wide characters", vec![markdown("日本語")], 4, 2 + 2),
            ("width zero", vec![markdown("abc")], 0, 3 + 2),
            (
                "markdown after a plain cell",
                vec![vec![
                    cell("by ", CellStyle::Dim),
                    cell("ab\ncd", CellStyle::Markdown),
                ]],
                10,
                2 + 2,
            ),
            (
                "mixed lines",
                vec![plain("x"), markdown("abcdefghijklmnopqrstu"), plain("y")],
                10,
                1 + 3 + 2 + 1,
            ),
        ];
        for (name, lines, width, rows) in cases {
            assert_eq!(display_rows(&lines, width), rows, "{name}");
        }
    }

    #[test]
    fn display_rows_of_headings_reach_the_rendered_rows() {
        use crate::app::tabs::cells::cell;
        let text = "The sidebar must open on the description of the pull request.";
        let body = format!(
            "## Why\n{text}\n\n## What\n{text}\n\n## Testing\n{text}\n\n## Verification\n{text}"
        );
        // The rendered rows of `ui::markdown::render`, measured.
        let cases: [(&str, &str, u16, usize); 4] = [
            (
                "four headings",
                "## Why\ntext\n## What\ntext\n## Testing\ntext\n## Verification\ntext",
                76,
                15,
            ),
            (
                "eight headings",
                "# A\n# B\n# C\n# D\n# E\n# F\n# G\n# H\ntext",
                76,
                17,
            ),
            ("body at width 76", &body, 76, 15),
            ("body at width 40", &body, 40, 19),
        ];
        for (name, text, width, rendered) in cases {
            let rows = display_rows(&[vec![cell(text, CellStyle::Markdown)]], width);
            assert!(rows >= rendered, "{name}: {rows} < {rendered}");
        }
    }
}
