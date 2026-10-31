//! Texture fills for empty areas.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;

use super::Theme;
use super::grunge::noise;

/// The texture glyphs.
const GLYPHS: [&str; 3] = ["░", "·", "˙"];

/// One cell in this many gets a glyph.
const ONE_IN: u64 = 9;

/// Fills the empty cells of an area with `░ · ˙` in a grey close to the
/// background. A cell that holds text keeps it, so a texture never sits
/// under text. With textures off, it draws nothing.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Texture {
    enabled: bool,
    color: Color,
    seed: u64,
}

impl Texture {
    /// Creates a texture for the theme.
    pub(crate) fn new(theme: &Theme) -> Self {
        Self {
            enabled: theme.textures(),
            color: theme.palette.surface,
            seed: 0,
        }
    }

    /// Sets the seed of the pattern.
    #[must_use]
    pub(crate) fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }
}

impl Widget for Texture {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !self.enabled {
            return;
        }
        let area = area.intersection(buf.area);
        let style = Style::new().fg(self.color);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                let n = noise(x, y, self.seed);
                if !n.is_multiple_of(ONE_IN) {
                    continue;
                }
                let Some(cell) = buf.cell_mut((x, y)) else {
                    continue;
                };
                if cell.symbol() != " " {
                    continue;
                }
                let index = usize::try_from((n / ONE_IN) % 3).unwrap_or(0);
                cell.set_symbol(GLYPHS[index]).set_style(style);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeKind;

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
    fn texture_fill() {
        let theme = Theme::new(ThemeKind::TrueColor, true, true);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 6));
        Texture::new(&theme).render(buf.area, &mut buf);
        insta::assert_snapshot!(text(&buf));
    }

    #[test]
    fn texture_keeps_text() {
        let theme = Theme::new(ThemeKind::TrueColor, true, true);
        let mut buf = Buffer::with_lines(["readable text that stays readable"]);
        Texture::new(&theme).render(buf.area, &mut buf);
        assert_eq!(text(&buf), "readable text that stays readable");
    }

    #[test]
    fn no_texture_when_off() {
        let theme = Theme::new(ThemeKind::Ansi16, true, true);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 6));
        Texture::new(&theme).render(buf.area, &mut buf);
        assert_eq!(buf, Buffer::empty(Rect::new(0, 0, 40, 6)));
    }
}
