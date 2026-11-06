//! The punk skin: a frame with angular cut corners and diagonal slashes,
//! ransom-note headers and slashed separators. No noise fill.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use super::grunge::noise;

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

/// A ransom-note header: the text in bold uppercase, each letter cut out
/// in one of three styles, with a trailing `╱` tail in `accent`. The
/// styles are black on `accent`, black on `white`, and `accent` on black
/// in italic. The spaces are black on `accent`. The same title always
/// gets the same styles, and two letters next to each other never share
/// a style.
pub(crate) fn ransom(title: &str, accent: Color, white: Color, black: Color) -> Line<'static> {
    let upper = title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase();
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let styles = [
        bold.bg(accent).fg(black),
        bold.bg(white).fg(black),
        bold.bg(black).fg(accent).add_modifier(Modifier::ITALIC),
    ];
    let seed = fnv1a(upper.as_bytes());
    let mut spans = vec![Span::styled(" ", styles[0])];
    let mut last = None;
    for (i, ch) in upper.chars().enumerate() {
        if ch == ' ' {
            spans.push(Span::styled(" ", styles[0]));
            continue;
        }
        let x = u16::try_from(i).unwrap_or(u16::MAX);
        let mut pick = usize::try_from(noise(x, 0, seed) % 3).unwrap_or(0);
        if last == Some(pick) {
            pick = (pick + 1) % 3;
        }
        last = Some(pick);
        spans.push(Span::styled(ch.to_string(), styles[pick]));
    }
    spans.push(Span::styled(" ", styles[0]));
    spans.push(Span::styled(SLASH, Style::new().fg(accent)));
    Line::from(spans)
}

/// The FNV-1a hash of `bytes`.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// A separator of `width` cells: a heavy line that ends in a slash.
pub(crate) fn separator(width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    format!("{}{SLASH}", HORIZONTAL.repeat(width - 1))
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
            .title(ransom("checks", Color::Red, Color::White, Color::Black))
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

    /// The three styles of a ransom header in red, white and black.
    fn ransom_styles() -> [Style; 3] {
        let bold = Style::new().add_modifier(Modifier::BOLD);
        [
            bold.bg(Color::Red).fg(Color::Black),
            bold.bg(Color::White).fg(Color::Black),
            bold.bg(Color::Black)
                .fg(Color::Red)
                .add_modifier(Modifier::ITALIC),
        ]
    }

    fn header(title: &str) -> Line<'static> {
        ransom(title, Color::Red, Color::White, Color::Black)
    }

    #[test]
    fn ransom_header_is_uppercase_with_a_tail() {
        let line = header("merge  #102");
        assert_eq!(line.to_string(), " MERGE #102 ╱");
        let [a, _, _] = ransom_styles();
        assert_eq!(line.spans[0].style, a, "the padding");
        let tail = line.spans.last().unwrap();
        assert_eq!(tail.content, "╱");
        assert_eq!(tail.style.fg, Some(Color::Red));
    }

    #[test]
    fn ransom_header_keeps_the_text() {
        for title in ["checks", "merge  #102", " help  keys ", "a b"] {
            let upper = title
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_uppercase();
            assert_eq!(header(title).to_string(), format!(" {upper} ╱"));
        }
    }

    #[test]
    fn ransom_header_is_the_same_twice() {
        assert_eq!(header("merge #102"), header("merge #102"));
    }

    #[test]
    fn ransom_letters_next_to_each_other_differ() {
        for title in ["checks", "merge #102", "conversation", "aaaaaaaa"] {
            let line = header(title);
            let inside = &line.spans[1..line.spans.len() - 2];
            for pair in inside.windows(2) {
                if pair[0].content != " " && pair[1].content != " " {
                    assert_ne!(pair[0].style, pair[1].style, "{title}");
                }
            }
        }
    }

    #[test]
    fn ransom_styles_are_a_b_or_c() {
        let styles = ransom_styles();
        let line = header("merge #102 conversation");
        for span in &line.spans[..line.spans.len() - 1] {
            assert!(styles.contains(&span.style), "{span:?}");
            if span.content == " " {
                assert_eq!(span.style, styles[0]);
            }
        }
    }

    #[test]
    fn separator_ends_in_a_slash() {
        assert_eq!(separator(5), "━━━━╱");
        assert_eq!(separator(0), "");
    }
}
