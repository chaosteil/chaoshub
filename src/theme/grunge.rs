//! Grunge borders: a border set of block glyphs, a block widget with a
//! rough frame, and stencil headers.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::symbols::border;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Widget};

use super::Theme;

/// The border set of block glyphs.
pub(crate) const GRUNGE: border::Set<'static> = border::Set {
    top_left: "▛",
    top_right: "▜",
    bottom_left: "▙",
    bottom_right: "▟",
    vertical_left: "▌",
    vertical_right: "▐",
    horizontal_top: "▀",
    horizontal_bottom: "▄",
};

/// The noise glyphs of a rough frame, from dense to light.
const NOISE: [&str; 3] = ["▓", "▒", "░"];

/// One edge cell in this many gets a noise glyph.
const NOISE_ONE_IN: u64 = 6;

/// A deterministic hash of a cell position and a seed. The same input
/// always gives the same value, so a frame does not flicker between draws.
pub(crate) fn noise(x: u16, y: u16, seed: u64) -> u64 {
    // splitmix64 over the packed input.
    let mut z = seed ^ (u64::from(x) << 32 | u64::from(y)) ^ 0x9e37_79b9_7f4a_7c15;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A header in the stencil style: a `▌` prefix, then the text in
/// uppercase, bold and letter-spaced.
pub(crate) fn stencil(text: &str, style: Style) -> Line<'static> {
    let spaced = text
        .split_whitespace()
        .map(|word| {
            word.to_uppercase()
                .chars()
                .map(String::from)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("   ");
    Line::from(vec![
        Span::styled("▌", style),
        Span::styled(format!(" {spaced}"), style.add_modifier(Modifier::BOLD)),
    ])
}

/// A block with a rough frame. With grunge on, the frame uses the block
/// glyphs of [`GRUNGE`] and some edge cells get a noise glyph. With grunge
/// off, it is a plain block with rounded borders.
#[derive(Debug, Clone)]
pub(crate) struct GrungeBlock<'a> {
    grunge: bool,
    title: Option<Line<'a>>,
    border_style: Style,
    style: Style,
    seed: u64,
}

impl<'a> GrungeBlock<'a> {
    /// Creates a block for the theme.
    pub(crate) fn new(theme: &Theme) -> Self {
        Self {
            grunge: theme.grunge(),
            title: None,
            border_style: Style::new().fg(theme.palette.fg_mute),
            style: Style::new().bg(theme.palette.bg),
            seed: 0,
        }
    }

    /// Sets the title on the top edge.
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

    /// Sets the seed of the noise, so two blocks get different frames.
    #[must_use]
    pub(crate) fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// The area inside the frame.
    pub(crate) fn inner(area: Rect) -> Rect {
        Rect {
            x: area.x.saturating_add(1),
            y: area.y.saturating_add(1),
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        }
    }

    /// The glyph of an edge cell: the set glyph, or a noise glyph for some
    /// cells. `rx` and `ry` are relative to the block.
    fn edge(&self, rx: u16, ry: u16, glyph: &'static str) -> &'static str {
        let n = noise(rx, ry, self.seed);
        if n.is_multiple_of(NOISE_ONE_IN) {
            let index = usize::try_from((n / NOISE_ONE_IN) % 3).unwrap_or(0);
            NOISE[index]
        } else {
            glyph
        }
    }

    /// Draws the rough frame.
    fn render_grunge(&self, area: Rect, buf: &mut Buffer) {
        buf.set_style(area, self.style);
        let right = area.width - 1;
        let bottom = area.height - 1;
        for ry in 0..area.height {
            for rx in 0..area.width {
                let glyph = match (rx, ry) {
                    (0, 0) => GRUNGE.top_left,
                    (x, 0) if x == right => GRUNGE.top_right,
                    (0, y) if y == bottom => GRUNGE.bottom_left,
                    (x, y) if x == right && y == bottom => GRUNGE.bottom_right,
                    (_, 0) => self.edge(rx, ry, GRUNGE.horizontal_top),
                    (_, y) if y == bottom => self.edge(rx, ry, GRUNGE.horizontal_bottom),
                    (0, _) => self.edge(rx, ry, GRUNGE.vertical_left),
                    (x, _) if x == right => self.edge(rx, ry, GRUNGE.vertical_right),
                    _ => continue,
                };
                if let Some(cell) = buf.cell_mut((area.x + rx, area.y + ry)) {
                    cell.set_symbol(glyph).set_style(self.border_style);
                }
            }
        }
        if let Some(title) = &self.title {
            let title_area = Rect {
                x: area.x + 2,
                y: area.y,
                width: area.width.saturating_sub(4),
                height: 1,
            };
            title.render(title_area, buf);
        }
    }
}

impl Widget for GrungeBlock<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 2 || area.height < 2 {
            return;
        }
        if self.grunge {
            self.render_grunge(area, buf);
            return;
        }
        let mut block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(self.border_style)
            .style(self.style);
        if let Some(title) = self.title {
            block = block.title(title);
        }
        block.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;
    use crate::theme::ThemeKind;

    fn draw(theme: &Theme, seed: u64) -> Buffer {
        let area = Rect::new(0, 0, 24, 6);
        let mut buf = Buffer::empty(area);
        GrungeBlock::new(theme)
            .title(stencil("checks", Style::new().fg(Color::Blue)))
            .seed(seed)
            .render(area, &mut buf);
        buf
    }

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

    #[test]
    fn noise_is_deterministic() {
        assert_eq!(noise(3, 4, 7), noise(3, 4, 7));
        assert_ne!(noise(3, 4, 7), noise(4, 3, 7));
        assert_ne!(noise(3, 4, 7), noise(3, 4, 8));
    }

    #[test]
    fn stencil_spaces_the_letters() {
        let line = stencil("open prs", Style::new());
        assert_eq!(line.to_string(), "▌ O P E N   P R S");
    }

    #[test]
    fn grunge_frame() {
        let theme = Theme::new(ThemeKind::TrueColor, true, true);
        insta::assert_snapshot!(text(&draw(&theme, 1)));
    }

    #[test]
    fn grunge_frame_other_seed() {
        let theme = Theme::new(ThemeKind::TrueColor, true, true);
        insta::assert_snapshot!(text(&draw(&theme, 2)));
    }

    #[test]
    fn grunge_frame_does_not_flicker() {
        let theme = Theme::new(ThemeKind::TrueColor, true, true);
        assert_eq!(draw(&theme, 1), draw(&theme, 1));
    }

    #[test]
    fn rounded_frame_without_grunge() {
        let theme = Theme::new(ThemeKind::Ansi16, true, true);
        insta::assert_snapshot!(text(&draw(&theme, 1)));
    }

    #[test]
    fn inner_is_inside_the_frame() {
        assert_eq!(
            GrungeBlock::inner(Rect::new(2, 3, 10, 5)),
            Rect::new(3, 4, 8, 3)
        );
    }
}
