//! The punk skin: a frame with angular cut corners and diagonal slashes,
//! inverted headers and slashed separators. No noise fill.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

/// The top left corner: a cut.
const TOP_LEFT: &str = "◢";
/// The top right corner: a cut.
const TOP_RIGHT: &str = "◥";
/// The bottom left corner.
const BOTTOM_LEFT: &str = "◤";
/// The bottom right corner.
const BOTTOM_RIGHT: &str = "◣";
/// The heavy top and bottom edges.
const HORIZONTAL: &str = "━";
/// The thick left edge.
const LEFT: &str = "▌";
/// The right edge.
const RIGHT: &str = "┃";
/// The diagonal accent at the top right.
const SLASH: &str = "╱";
/// The number of slashes in the accent.
const SLASHES: u16 = 3;
/// The cells between the accent and the top right corner.
const SLASH_GAP: u16 = 1;

/// An inverted header: the text in bold uppercase on a block of `color`,
/// in `text` color, with a trailing `╱` tail in `color`.
pub(crate) fn inverted(title: &str, color: Color, text: Color) -> Line<'static> {
    let upper = title.split_whitespace().collect::<Vec<_>>().join(" ");
    Line::from(vec![
        Span::styled(
            format!(" {} ", upper.to_uppercase()),
            Style::new().bg(color).fg(text).add_modifier(Modifier::BOLD),
        ),
        Span::styled(SLASH, Style::new().fg(color)),
    ])
}

/// A separator of `width` cells: a heavy line that ends in a slash.
pub(crate) fn separator(width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    format!("{}{SLASH}", HORIZONTAL.repeat(width - 1))
}

/// Moves the symbol and the style of each cell in `area` to the right by
/// `cells`. The cells that go past the right edge of `area` are dropped.
/// The free cells on the left get a space in the `fill` style.
pub(crate) fn shift_right(buf: &mut Buffer, area: Rect, cells: u16, fill: Style) {
    let area = area.intersection(buf.area);
    if cells == 0 || area.is_empty() {
        return;
    }
    let free = area.left().saturating_add(cells);
    for y in area.top()..area.bottom() {
        for x in (area.left()..area.right()).rev() {
            if x >= free {
                let moved = buf[(x - cells, y)].clone();
                buf[(x, y)] = moved;
            } else {
                buf[(x, y)].reset();
                buf[(x, y)].set_symbol(" ").set_style(fill);
            }
        }
    }
}

/// A block with the punk frame: a thick left edge, a heavy top line with
/// cut corners and a diagonal accent at the top right, and a heavy
/// bottom line. The inside is plain.
#[derive(Debug, Clone, Default)]
pub(crate) struct PunkBlock<'a> {
    title: Option<Line<'a>>,
    border_style: Style,
    style: Style,
}

impl<'a> PunkBlock<'a> {
    /// Sets the header on the top edge.
    #[must_use]
    pub(crate) fn title(mut self, title: impl Into<Line<'a>>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Sets the style of the frame.
    #[must_use]
    pub(crate) fn border_style(mut self, style: Style) -> Self {
        self.border_style = style;
        self
    }

    /// Sets the style of the whole area.
    #[must_use]
    pub(crate) fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    /// True when the top edge cell `rx` of a block `width` cells wide is
    /// a slash of the accent.
    fn is_slash(rx: u16, width: u16) -> bool {
        let end = width.saturating_sub(1 + SLASH_GAP);
        let start = end.saturating_sub(SLASHES);
        width >= 4 * SLASHES && rx >= start && rx < end
    }
}

impl Widget for PunkBlock<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 2 || area.height < 2 {
            return;
        }
        buf.set_style(area, self.style);
        let right = area.width - 1;
        let bottom = area.height - 1;
        for ry in 0..area.height {
            for rx in 0..area.width {
                let glyph = match (rx, ry) {
                    (0, 0) => TOP_LEFT,
                    (x, 0) if x == right => TOP_RIGHT,
                    (0, y) if y == bottom => BOTTOM_LEFT,
                    (x, y) if x == right && y == bottom => BOTTOM_RIGHT,
                    (x, 0) if Self::is_slash(x, area.width) => SLASH,
                    (_, 0) => HORIZONTAL,
                    (_, y) if y == bottom => HORIZONTAL,
                    (0, _) => LEFT,
                    (x, _) if x == right => RIGHT,
                    _ => continue,
                };
                if let Some(cell) = buf.cell_mut((area.x + rx, area.y + ry)) {
                    cell.set_symbol(glyph).set_style(self.border_style);
                }
            }
        }
        if let Some(title) = &self.title {
            // The header stays clear of the accent and the corner.
            let room = area.width.saturating_sub(4 + SLASHES + SLASH_GAP);
            let title_area = Rect::new(area.x + 2, area.y, room, 1);
            title.render(title_area, buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(buf: &Buffer) -> String {
        let area = buf.area;
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn draw(width: u16, height: u16) -> Buffer {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        PunkBlock::default()
            .title(inverted("checks", Color::Red, Color::Black))
            .border_style(Style::new().fg(Color::Red))
            .render(area, &mut buf);
        buf
    }

    #[test]
    fn punk_frame() {
        insta::assert_snapshot!(text(&draw(28, 5)));
    }

    #[test]
    fn narrow_frame_has_no_accent() {
        let buf = draw(10, 3);
        assert!(!text(&buf).contains(SLASH.repeat(3).as_str()));
    }

    #[test]
    fn inverted_header_is_uppercase_on_the_color() {
        let line = inverted("merge  #102", Color::Red, Color::Black);
        assert_eq!(line.to_string(), " MERGE #102 ╱");
        assert_eq!(line.spans[0].style.bg, Some(Color::Red));
        assert_eq!(line.spans[0].style.fg, Some(Color::Black));
        assert!(line.spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(line.spans[1].style.fg, Some(Color::Red));
    }

    #[test]
    fn shift_right_moves_the_cells() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 8, 1));
        buf.set_string(0, 0, "abcdefgh", Style::new().fg(Color::Red));
        let fill = Style::new().bg(Color::Blue);
        shift_right(&mut buf, Rect::new(1, 0, 5, 1), 2, fill);
        assert_eq!(text(&buf), "a  bcdgh");
        assert_eq!(buf[(1, 0)].bg, Color::Blue);
        assert_eq!(buf[(2, 0)].bg, Color::Blue);
        assert_eq!(buf[(3, 0)].fg, Color::Red);
        assert_eq!(buf[(3, 0)].bg, Color::Reset);
        // A shift wider than the area fills it, and outside the buffer
        // it does nothing.
        shift_right(&mut buf, Rect::new(6, 0, 2, 1), 5, fill);
        assert_eq!(text(&buf), "a  bcd  ");
        shift_right(&mut buf, Rect::new(20, 0, 2, 1), 1, fill);
    }

    #[test]
    fn separator_ends_in_a_slash() {
        assert_eq!(separator(5), "━━━━╱");
        assert_eq!(separator(0), "");
    }
}
