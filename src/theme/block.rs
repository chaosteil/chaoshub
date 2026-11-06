//! The framed block of the theme: the skin picks the frame and the
//! header style.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;

use super::grunge::GrungeBlock;
use super::punk::PunkBlock;
use super::{Skin, Theme};

/// A block with the frame and the header of the theme skin. The header
/// and the frame take the accent color; `border` sets another frame
/// color.
#[derive(Debug, Clone)]
pub(crate) struct ThemeBlock<'a> {
    theme: Theme,
    title: &'a str,
    accent: Color,
    border: Option<Color>,
    seed: u64,
}

impl<'a> ThemeBlock<'a> {
    /// Creates a block with a header. An empty title gives no header.
    pub(crate) fn new(theme: &Theme, title: &'a str) -> Self {
        Self {
            theme: *theme,
            title,
            accent: theme.palette.fg_mute,
            border: None,
            seed: 0,
        }
    }

    /// Sets the color of the header and the frame.
    #[must_use]
    pub(crate) fn accent(mut self, color: Color) -> Self {
        self.accent = color;
        self
    }

    /// Sets a frame color that differs from the header color.
    #[must_use]
    pub(crate) fn border(mut self, color: Color) -> Self {
        self.border = Some(color);
        self
    }

    /// Sets the seed of the frame noise, so two blocks get different
    /// frames.
    #[must_use]
    pub(crate) fn seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// The area inside the frame.
    pub(crate) fn inner(area: Rect) -> Rect {
        GrungeBlock::inner(area)
    }
}

impl Widget for ThemeBlock<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let p = &self.theme.palette;
        let border = self.border.unwrap_or(self.accent);
        if self.theme.skin == Skin::Punk {
            let mut block = PunkBlock::default()
                .border_style(Style::new().fg(border).bg(p.bg))
                .style(Style::new().bg(p.bg).fg(p.fg));
            if !self.title.is_empty() {
                block = block.title(self.theme.header(self.title, self.accent));
            }
            block.render(area, buf);
            return;
        }
        let mut block = GrungeBlock::new(&self.theme)
            .border_style(Style::new().fg(border).bg(p.bg))
            .style(Style::new().bg(p.bg).fg(p.fg))
            .seed(self.seed);
        if !self.title.is_empty() {
            block = block.title(self.theme.header(self.title, self.accent));
        }
        block.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Palette, ThemeKind};

    fn draw(theme: &Theme) -> Buffer {
        let area = Rect::new(0, 0, 30, 4);
        let mut buf = Buffer::empty(area);
        theme
            .block("help")
            .accent(theme.palette.primary)
            .render(area, &mut buf);
        buf
    }

    #[test]
    fn punk_block_has_the_punk_frame_and_header() {
        let theme = Theme::punk(ThemeKind::TrueColor);
        let p = theme.palette;
        assert_eq!(p, Palette::pierre());
        let buf = draw(&theme);
        assert_eq!(buf[(0, 0)].symbol(), "◢");
        assert_eq!(buf[(0, 0)].fg, p.primary);
        assert_eq!(buf[(3, 0)].symbol(), "H");
        let colors = (buf[(3, 0)].bg, buf[(3, 0)].fg);
        assert!(
            [(p.primary, p.bg), (p.fg, p.bg), (p.bg, p.primary)].contains(&colors),
            "{colors:?}"
        );
    }

    #[test]
    fn grunge_block_has_the_grunge_frame() {
        let theme = Theme::new(ThemeKind::TrueColor, true, true);
        let buf = draw(&theme);
        assert_eq!(buf[(0, 0)].symbol(), "▛");
        assert_eq!(buf[(2, 0)].symbol(), "▌");
    }
}
