//! The picker popup: a list of items with a cursor, for example the
//! author filter. The list scrolls so the cursor stays visible.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::theme::Theme;
use crate::theme::ThemeBlock;
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
    theme
        .block(title)
        .accent(p.primary)
        .seed(23)
        .render(area, buf);
    let inner = ThemeBlock::inner(area);
    if inner.height == 0 || inner.width < 2 {
        return;
    }
    let rows = usize::from(inner.height);
    let offset = cursor.saturating_sub(rows - 1);
    let selection = theme.selected_row();
    let tail = u16::from(selection.tail.is_some());
    let width = usize::from(inner.width.saturating_sub(2 + tail));
    for (y, (index, item)) in (inner.y..).zip(items.iter().enumerate().skip(offset).take(rows)) {
        let line = Rect::new(inner.x, y, inner.width, 1);
        if index == cursor {
            let style = Style::new().fg(p.fg).patch(selection.patch);
            buf.set_style(line, style);
            buf.set_string(inner.x, y, selection.marker, style.fg(selection.marker_fg));
            if let Some((glyph, tail)) = selection.tail {
                buf.set_string(line.right() - 1, y, glyph, tail);
            }
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
