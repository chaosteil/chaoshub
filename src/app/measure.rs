//! The screen rows of the pane models. `app` scrolls the panes by these
//! rows, and `ui` draws the panes with the same widths. Thus the two
//! never disagree.

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
    fn the_shift_of_the_selected_row_narrows_the_tree() {
        let nodes = [node(0, vec![cell("abcdefghij", CellStyle::Markdown)])];
        assert_eq!(tree_rows(&nodes, 13, 0), [1]);
        assert_eq!(tree_rows(&nodes, 13, 1), [2]);
    }
}
