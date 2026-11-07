//! The text selection of a mouse drag: the marked cells of a text area,
//! the text of these cells, and their highlight.
//!
//! The selection uses reading order, as a terminal does: from the start
//! cell to the end of its row, then full rows, then to the end cell. It
//! stays inside the text area where it started.

use std::ops::Range;

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::Modifier;

use crate::ui::text;

/// A selection in a text area: the cell of the press and the cell of the
/// last drag. Both cells are marked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TextSelection {
    /// The text area where the selection started.
    pub area: Rect,
    pub start: Position,
    pub end: Position,
}

impl TextSelection {
    /// A selection of the one cell `at` in `area`. A cell out of `area`
    /// moves to the nearest cell of `area`.
    pub(crate) fn new(area: Rect, at: Position) -> Self {
        let at = clamp(area, at);
        Self {
            area,
            start: at,
            end: at,
        }
    }

    /// Moves the end of the selection to `at`. A cell out of the area
    /// moves to the nearest cell of the area.
    pub(crate) fn extend(&mut self, at: Position) {
        self.end = clamp(self.area, at);
    }

    /// True when the end is not the start: the mouse moved after the
    /// press.
    pub(crate) fn moved(&self) -> bool {
        self.start != self.end
    }

    /// The marked columns of each row, top row first. The first row starts
    /// at the earlier cell, the last row ends at the later cell, and the
    /// rows between them are full.
    pub(crate) fn rows(&self) -> Vec<(u16, Range<u16>)> {
        let (first, last) = if (self.start.y, self.start.x) <= (self.end.y, self.end.x) {
            (self.start, self.end)
        } else {
            (self.end, self.start)
        };
        (first.y..=last.y)
            .map(|y| {
                let from = if y == first.y {
                    first.x
                } else {
                    self.area.left()
                };
                let to = if y == last.y {
                    last.x + 1
                } else {
                    self.area.right()
                };
                (y, from..to)
            })
            .collect()
    }
}

/// The nearest cell of `area` to `at`.
fn clamp(area: Rect, at: Position) -> Position {
    Position {
        x: at.x.min(area.right().saturating_sub(1)).max(area.left()),
        y: at.y.min(area.bottom().saturating_sub(1)).max(area.top()),
    }
}

/// The text of the marked cells of `buf`: one line per row, with the
/// spaces at the end of each line and the empty lines at the end cut.
/// A wide symbol covers more than one cell, so the cells that it covers
/// are skipped.
pub(crate) fn text(buf: &Buffer, sel: &TextSelection) -> String {
    let mut lines: Vec<String> = sel
        .rows()
        .into_iter()
        .map(|(y, columns)| {
            let mut line = String::new();
            let mut x = columns.start;
            while x < columns.end {
                let Some(cell) = buf.cell(Position { x, y }) else {
                    break;
                };
                let symbol = cell.symbol();
                line.push_str(symbol);
                let width = u16::try_from(text::width(symbol).max(1)).unwrap_or(1);
                x = x.saturating_add(width);
            }
            line.trim_end_matches(' ').to_owned()
        })
        .collect();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

/// Shows the marked cells of `buf` reversed.
pub(crate) fn highlight(buf: &mut Buffer, sel: &TextSelection) {
    for (y, columns) in sel.rows() {
        for x in columns {
            if let Some(cell) = buf.cell_mut(Position { x, y }) {
                cell.modifier.insert(Modifier::REVERSED);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: u16, y: u16) -> Position {
        Position { x, y }
    }

    /// A buffer of 10 x 4 with `lines` from the top left cell.
    fn buffer(lines: &[&str]) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 4));
        for (y, line) in (0..).zip(lines) {
            buf.set_string(0, y, line, ratatui::style::Style::new());
        }
        buf
    }

    /// The text area of the tests: columns 1 to 8, rows 0 to 2.
    const AREA: Rect = Rect::new(1, 0, 8, 3);

    fn drag(from: Position, to: Position) -> TextSelection {
        let mut sel = TextSelection::new(AREA, from);
        sel.extend(to);
        sel
    }

    #[test]
    fn a_forward_drag_reads_in_reading_order() {
        let sel = drag(at(3, 0), at(2, 2));
        assert_eq!(sel.rows(), [(0, 3..9), (1, 1..9), (2, 1..3)]);
        let buf = buffer(&["|abcdefgh|", "|ijklmnop|", "|qrstuvwx|"]);
        assert_eq!(text(&buf, &sel), "cdefgh\nijklmnop\nqr");
    }

    #[test]
    fn a_backward_drag_marks_the_same_cells() {
        assert_eq!(
            drag(at(2, 2), at(3, 0)).rows(),
            drag(at(3, 0), at(2, 2)).rows()
        );
        assert_eq!(drag(at(6, 1), at(2, 1)).rows(), [(1, 2..7)]);
    }

    #[test]
    fn a_one_row_drag_and_a_press_without_a_move() {
        let buf = buffer(&["|abcdefgh|"]);
        let sel = drag(at(2, 0), at(4, 0));
        assert!(sel.moved());
        assert_eq!(text(&buf, &sel), "bcd");
        let press = TextSelection::new(AREA, at(2, 0));
        assert!(!press.moved());
        assert_eq!(press.rows(), [(0, 2..3)]);
    }

    #[test]
    fn a_drag_out_of_the_area_stops_at_its_edge() {
        let sel = drag(at(4, 1), at(0, 9));
        assert_eq!(sel.end, at(1, 2));
        let sel = drag(at(4, 1), at(30, 0));
        assert_eq!(sel.end, at(8, 0));
        let buf = buffer(&["|abcdefgh|", "|ijklmnop|"]);
        assert_eq!(text(&buf, &sel), "h\nijkl", "no border cell");
    }

    #[test]
    fn the_spaces_and_the_empty_rows_at_the_end_are_cut() {
        let buf = buffer(&["|ab      |", "|        |", "|        |"]);
        assert_eq!(text(&buf, &drag(at(1, 0), at(8, 2))), "ab");
        let buf = buffer(&["|ab      |", "|        |", "|cd      |"]);
        assert_eq!(text(&buf, &drag(at(1, 0), at(8, 2))), "ab\n\ncd");
    }

    #[test]
    fn a_wide_character_shows_once() {
        let buf = buffer(&["|a日本b   |"]);
        assert_eq!(text(&buf, &drag(at(1, 0), at(8, 0))), "a日本b");
    }

    #[test]
    fn the_marked_cells_show_reversed() {
        let mut buf = buffer(&["|abcdefgh|", "|ijklmnop|"]);
        highlight(&mut buf, &drag(at(7, 0), at(2, 1)));
        let reversed = |x, y| {
            buf.cell(at(x, y))
                .is_some_and(|cell| cell.modifier.contains(Modifier::REVERSED))
        };
        assert!(reversed(7, 0));
        assert!(reversed(8, 0));
        assert!(!reversed(9, 0), "the border stays");
        assert!(reversed(1, 1));
        assert!(reversed(2, 1));
        assert!(!reversed(3, 1));
        assert!(!reversed(6, 0));
    }
}
