//! The help overlay: the key bindings in sections, at the right edge of
//! the screen. The transitions slide it in from the right.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::app::models::HelpModel;
use crate::theme::Theme;
use crate::theme::grunge::{GrungeBlock, stencil};
use crate::ui::text;

/// The minimum width of the key column.
const KEY_MIN: usize = 6;

/// Draws the help overlay into its area.
pub(crate) fn render(model: &HelpModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let p = &theme.palette;
    let area = area.intersection(buf.area);
    Clear.render(area, buf);
    GrungeBlock::new(theme)
        .title(stencil("help", Style::new().fg(p.info)))
        .border_style(Style::new().fg(p.info).bg(p.bg))
        .style(Style::new().bg(p.bg).fg(p.fg))
        .seed(11)
        .render(area, buf);
    let inner = GrungeBlock::inner(area);
    let inner = Rect::new(
        inner.x + 1,
        inner.y + 1,
        inner.width.saturating_sub(2),
        inner.height.saturating_sub(1),
    );
    let width = usize::from(inner.width);
    let mut y = inner.y;
    for (i, section) in model.sections.iter().enumerate() {
        if i > 0 {
            y += 1;
        }
        if y >= inner.bottom() {
            return;
        }
        stencil(&section.title, Style::new().fg(p.primary))
            .render(Rect::new(inner.x, y, inner.width, 1), buf);
        y += 1;
        let key_width = section
            .entries
            .iter()
            .map(|(key, _)| text::width(key))
            .max()
            .unwrap_or(0)
            .max(KEY_MIN);
        for (key, label) in &section.entries {
            if y >= inner.bottom() {
                return;
            }
            let key_text = text::fit(key, key_width.min(width), crate::app::models::Align::Left);
            buf.set_string(
                inner.x,
                y,
                &key_text,
                Style::new().fg(p.primary).add_modifier(Modifier::BOLD),
            );
            let label_x = inner.x + u16::try_from(key_width + 1).unwrap_or(inner.width);
            if label_x < inner.right() {
                let room = usize::from(inner.right() - label_x);
                buf.set_string(
                    label_x,
                    y,
                    text::truncate(label, room),
                    Style::new().fg(p.fg),
                );
            }
            y += 1;
        }
    }
}
