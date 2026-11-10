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
            shadow(area, buf, p.primary);
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

/// Draws a drop shadow in `color` under a block: the column just right of
/// `area` and the row just below it, one cell away from the top and the
/// left. The shadow is clipped to the buffer.
fn shadow(area: Rect, buf: &mut Buffer, color: Color) {
    if area.is_empty() {
        return;
    }
    let column = (area.y.saturating_add(1)..=area.bottom()).map(|y| (area.right(), y));
    let row = (area.x.saturating_add(1)..=area.right()).map(|x| (x, area.bottom()));
    for position in column.chain(row) {
        if let Some(cell) = buf.cell_mut(position) {
            cell.set_symbol(" ").set_style(Style::new().bg(color));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Palette, PaletteChoice, ThemeKind, ThemeStyle};

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
        assert_eq!(buf[(3, 0)].bg, p.primary);
        assert_eq!(buf[(3, 0)].fg, p.bg);
    }

    #[test]
    fn punk_block_has_a_drop_shadow() {
        let theme = Theme::punk(ThemeKind::TrueColor);
        let p = theme.palette;
        let area = Rect::new(2, 1, 10, 4);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 10));
        theme.block("help").accent(p.danger).render(area, &mut buf);
        // The column right of the block, from one row down to the row
        // below the block.
        for y in 2..=5 {
            assert_eq!(buf[(12, y)].symbol(), " ", "y {y}");
            assert_eq!(buf[(12, y)].bg, p.primary, "y {y}");
        }
        // The row below the block, from one column in.
        for x in 3..=12 {
            assert_eq!(buf[(x, 5)].symbol(), " ", "x {x}");
            assert_eq!(buf[(x, 5)].bg, p.primary, "x {x}");
        }
        // The top-right and the bottom-left cells next to the block stay
        // empty.
        assert_eq!(buf[(12, 1)], ratatui::buffer::Cell::default());
        assert_eq!(buf[(2, 5)], ratatui::buffer::Cell::default());
        // Nothing past the shadow.
        assert_eq!(buf[(13, 3)], ratatui::buffer::Cell::default());
        assert_eq!(buf[(5, 6)], ratatui::buffer::Cell::default());
    }

    #[test]
    fn punk_block_at_the_edge_does_not_panic() {
        let theme = Theme::punk(ThemeKind::TrueColor);
        let area = Rect::new(0, 0, 30, 4);
        let mut buf = Buffer::empty(area);
        theme.block("help").render(area, &mut buf);
        let area = Rect::new(25, 2, 5, 2);
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 4));
        theme.block("").render(area, &mut buf);
        assert_eq!(buf[(26, 0)], ratatui::buffer::Cell::default());
    }

    #[test]
    fn grunge_block_has_the_grunge_frame() {
        let theme = Theme::styled(
            ThemeKind::TrueColor,
            ThemeStyle::Grunge,
            PaletteChoice::Pierre,
            true,
            true,
        );
        let buf = draw(&theme);
        assert_eq!(buf[(0, 0)].symbol(), "▛");
        assert_eq!(buf[(2, 0)].symbol(), "▌");
    }
}
