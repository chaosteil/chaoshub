//! The screen rows of the pane models. `app` scrolls the panes by these
//! rows, and `ui` draws the panes with the same widths and the same
//! wrap. Thus the two never disagree.

use std::ops::Range;

use crate::app::models::{Cell, CellStyle, TreeNode};
use crate::markup::{self, Source};

/// The width of the cursor gutter of a list or a tree.
pub(crate) const GUTTER: u16 = 1;
/// The cells of indent per tree depth.
pub(crate) const TREE_INDENT: usize = 2;
/// The cells of the expand marker of a tree node, for example `▸ `.
const TREE_MARKER: usize = 2;

/// The width for the text of the tree nodes in a pane of `width` cells:
/// the pane without the gutter and without the `shift` cells of the
/// selected row.
pub(crate) fn tree_text_width(width: u16, shift: u16) -> u16 {
    width.saturating_sub(GUTTER).saturating_sub(shift)
}

/// The cells before the text of a node at `depth`: the indent and the
/// expand marker.
pub(crate) fn node_prefix(depth: u8) -> usize {
    TREE_INDENT * usize::from(depth) + TREE_MARKER
}

/// The index of the markdown or HTML cell of a line, if it has one.
pub(crate) fn markup_cell(cells: &[Cell]) -> Option<usize> {
    cells
        .iter()
        .position(|c| matches!(c.style, CellStyle::Markdown | CellStyle::Html))
}

/// The source of a markdown or HTML cell.
pub(crate) fn source(cell: &Cell) -> Source<'_> {
    if cell.style == CellStyle::Html {
        Source::Html(&cell.text)
    } else {
        Source::Markdown(&cell.text)
    }
}

/// The display width of the texts of `cells`.
fn cells_width(cells: &[Cell]) -> usize {
    cells
        .iter()
        .map(|c| textwrap::core::display_width(&c.text))
        .sum()
}

/// The rows of a tree node with the text width `width`. A node without a
/// markdown or HTML cell takes one row. A node with one takes the rows of
/// `markup::rows` at the width after the prefix and the cells before it,
/// at least one.
pub(crate) fn node_rows(node: &TreeNode, width: u16) -> usize {
    let width = usize::from(width).saturating_sub(node_prefix(node.depth));
    let Some(at) = markup_cell(&node.cells) else {
        return 1;
    };
    let width = width.saturating_sub(cells_width(&node.cells[..at]));
    markup::rows(source(&node.cells[at]), width.max(1))
        .len()
        .max(1)
}

/// The rows of each node of a tree in a pane of `width` cells, with the
/// `shift` of the selected row.
pub(crate) fn tree_rows(nodes: &[TreeNode], width: u16, shift: u16) -> Vec<usize> {
    let width = tree_text_width(width, shift);
    nodes.iter().map(|node| node_rows(node, width)).collect()
}

/// The display width of a char in cells.
fn char_width(c: char) -> usize {
    textwrap::core::display_width(c.encode_utf8(&mut [0; 4]))
}

/// The byte ranges of the rows of `text` wrapped at word boundaries to
/// `width` cells. A row breaks after its last space that fits. The space
/// at a break goes away. A word that is wider than `width` breaks at
/// the last char that fits. A text without a break, the empty text too,
/// is one row.
pub(crate) fn wrap_ranges(text: &str, width: usize) -> Vec<Range<usize>> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut start = 0;
    let mut used = 0;
    // The byte after the last space of the row, and the width up to it.
    let mut space: Option<(usize, usize)> = None;
    for (i, c) in text.char_indices() {
        let w = char_width(c);
        if used + w > width && i > start {
            if c == ' ' {
                rows.push(start..i);
                start = i + 1;
                used = 0;
                space = None;
                continue;
            }
            match space.take() {
                Some((at, at_width)) if used - at_width + w <= width => {
                    rows.push(start..at);
                    start = at;
                    used -= at_width;
                }
                _ => {
                    rows.push(start..i);
                    start = i;
                    used = 0;
                }
            }
        }
        used += w;
        if c == ' ' {
            space = Some((i + 1, used));
        }
    }
    rows.push(start..text.len());
    rows
}

/// The text of `cells`.
fn cells_text(cells: &[Cell]) -> String {
    cells.iter().map(|c| c.text.as_str()).collect()
}

/// The texts of the display lines of one line of cells at `width`, as
/// `ui` draws them before the wrap. A line without a markdown or HTML
/// cell is one display line. A markdown or HTML cell becomes the rows of
/// `markup::rows` at the width after the cells before it. The next rows
/// are indented under the first, and the cells after it end the last
/// row.
pub(crate) fn line_texts(cells: &[Cell], width: u16) -> Vec<String> {
    let Some(at) = markup_cell(cells) else {
        return vec![cells_text(cells)];
    };
    let before = cells_text(&cells[..at]);
    let after = cells_text(&cells[at + 1..]);
    let indent = textwrap::core::display_width(&before);
    let body_width = usize::from(width).saturating_sub(indent).max(1);
    let rows = markup::rows(source(&cells[at]), body_width);
    let mut texts: Vec<String> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let lead = if i == 0 {
                before.clone()
            } else {
                " ".repeat(indent)
            };
            row.iter().fold(lead, |text, piece| text + &piece.text)
        })
        .collect();
    if texts.is_empty() {
        texts.push(before);
    }
    if let Some(last) = texts.last_mut() {
        last.push_str(&after);
    }
    texts
}

/// The screen rows of the lines of a pane of lines at `width`: the
/// display lines of each line, each wrapped to `width` when `wrap` is
/// true.
pub(crate) fn display_rows(lines: &[Vec<Cell>], wrap: bool, width: u16) -> usize {
    lines
        .iter()
        .flat_map(|cells| line_texts(cells, width))
        .map(|text| {
            if wrap {
                wrap_ranges(&text, usize::from(width)).len()
            } else {
                1
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tabs::cells::cell;

    fn node(depth: u8, cells: Vec<Cell>) -> TreeNode {
        TreeNode {
            depth,
            expanded: None,
            cells,
        }
    }

    #[test]
    fn node_rows_cases() {
        let plain = |text: &str| vec![cell(text, CellStyle::Normal)];
        let markdown = |text: &str| vec![cell(text, CellStyle::Markdown)];
        // The text width of each case is 12: 10 cells after the prefix
        // of a node at depth 0.
        let cases: Vec<(&str, TreeNode, usize)> = vec![
            ("a plain node", node(0, plain("a text that is too wide")), 1),
            ("a short body", node(0, markdown("abc")), 1),
            ("exact width", node(0, markdown("abcdefghij")), 1),
            ("one row more", node(0, markdown("abcdefghijk")), 2),
            ("a blank source line", node(0, markdown("a\n\nb")), 3),
            ("one level deeper", node(1, markdown("abcdefghij")), 2),
            (
                "a body after a plain cell",
                node(
                    0,
                    vec![
                        cell("by ", CellStyle::Dim),
                        cell("abcdefgh", CellStyle::Markdown),
                    ],
                ),
                2,
            ),
            ("an empty body", node(0, markdown("")), 1),
        ];
        for (name, node, rows) in cases {
            assert_eq!(node_rows(&node, 12), rows, "{name}");
        }
    }

    #[test]
    fn wrap_ranges_cases() {
        let cases: [(&str, &str, usize, &[&str]); 9] = [
            ("empty", "", 5, &[""]),
            ("fits", "one two", 7, &["one two"]),
            (
                "a break after a space",
                "one two three",
                5,
                &["one ", "two ", "three"],
            ),
            ("the space at the break goes", "one two", 3, &["one", "two"]),
            ("a long word", "abcdefgh", 3, &["abc", "def", "gh"]),
            (
                "a long word after a word",
                "a bcdefgh",
                4,
                &["a ", "bcde", "fgh"],
            ),
            ("wide chars", "日本語", 4, &["日本", "語"]),
            ("leading spaces stay", "  ab cd", 4, &["  ab", "cd"]),
            ("width zero is one cell", "ab", 0, &["a", "b"]),
        ];
        for (name, text, width, rows) in cases {
            let got: Vec<&str> = wrap_ranges(text, width)
                .into_iter()
                .map(|range| &text[range])
                .collect();
            assert_eq!(got, rows, "{name}");
        }
    }

    #[test]
    fn display_rows_cases() {
        let plain = |text: &str| vec![cell(text, CellStyle::Normal)];
        let markdown = |text: &str| vec![cell(text, CellStyle::Markdown)];
        let cases: Vec<(&str, Vec<Vec<Cell>>, u16, usize)> = vec![
            ("no lines", Vec::new(), 10, 0),
            (
                "plain lines",
                vec![plain("a"), plain("b c d e f g h")],
                4,
                1 + 4,
            ),
            (
                "a plain line that wraps",
                vec![plain("one two three")],
                5,
                3,
            ),
            ("empty line", vec![Vec::new()], 10, 1),
            ("one short line", vec![markdown("abc")], 10, 1),
            ("exact width", vec![markdown("abcdefghij")], 10, 1),
            ("one row more", vec![markdown("abcdefghijk")], 10, 2),
            ("blank source line", vec![markdown("a\n\nb")], 10, 3),
            ("wide characters", vec![markdown("日本語")], 4, 2),
            ("width zero", vec![markdown("abc")], 0, 3),
            (
                "markdown after a plain cell",
                vec![vec![
                    cell("by ", CellStyle::Dim),
                    cell("ab\ncd", CellStyle::Markdown),
                ]],
                10,
                1,
            ),
            (
                "mixed lines",
                vec![plain("x"), markdown("abcdefghijklmnopqrstu"), plain("y")],
                10,
                1 + 3 + 1,
            ),
        ];
        for (name, lines, width, rows) in cases {
            assert_eq!(display_rows(&lines, true, width), rows, "{name}");
        }
        let long = vec![plain("a text that is too wide")];
        assert_eq!(display_rows(&long, false, 5), 1, "no wrap");
    }

    #[test]
    fn display_rows_of_headings_are_the_rendered_rows() {
        let text = "The sidebar must open on the description of the pull request.";
        let body = format!(
            "## Why\n{text}\n\n## What\n{text}\n\n## Testing\n{text}\n\n## Verification\n{text}"
        );
        // The rendered rows of `ui::markdown::render`.
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
            let rows = display_rows(&[vec![cell(text, CellStyle::Markdown)]], true, width);
            assert_eq!(rows, rendered, "{name}");
        }
    }

    #[test]
    fn the_shift_of_the_selected_row_narrows_the_tree() {
        let nodes = [node(0, vec![cell("abcdefghij", CellStyle::Markdown)])];
        assert_eq!(tree_rows(&nodes, 13, 0), [1]);
        assert_eq!(tree_rows(&nodes, 13, 1), [2]);
    }
}
