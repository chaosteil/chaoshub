//! The picker popup: a list of items with a cursor, for example the
//! author filter. The list scrolls so the cursor stays visible.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::theme::Theme;
use crate::theme::grunge::{GrungeBlock, stencil};
use crate::ui::text;

/// Draws the picker popup into its area.
pub(crate) fn render(
    title: &str,
    items: &[String],
    cursor: usize,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let p = &theme.palette;
    let area = area.intersection(buf.area);
    Clear.render(area, buf);
    GrungeBlock::new(theme)
        .title(stencil(title, Style::new().fg(p.primary)))
        .border_style(Style::new().fg(p.primary).bg(p.bg))
        .style(Style::new().bg(p.bg).fg(p.fg))
        .seed(23)
        .render(area, buf);
    let inner = GrungeBlock::inner(area);
    if inner.height == 0 || inner.width < 2 {
        return;
    }
    let rows = usize::from(inner.height);
    let offset = cursor.saturating_sub(rows - 1);
    let width = usize::from(inner.width - 2);
    for (y, (index, item)) in (inner.y..).zip(items.iter().enumerate().skip(offset).take(rows)) {
        let line = Rect::new(inner.x, y, inner.width, 1);
        if index == cursor {
            let style = Style::new().bg(p.surface).fg(p.fg);
            buf.set_style(line, style);
            buf.set_string(inner.x, y, "▌", style.fg(p.primary));
            buf.set_string(
                inner.x + 2,
                y,
                text::truncate(item, width),
                style.add_modifier(Modifier::BOLD),
            );
        } else {
            buf.set_string(
                inner.x + 2,
                y,
                text::truncate(item, width),
                Style::new().fg(p.fg_dim),
            );
        }
    }
}
