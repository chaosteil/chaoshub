//! The confirm popup: a title, the lines of the action, and the keys. A
//! dangerous action (merge, close) gets the danger color.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};

use crate::bus::ConfirmSpec;
use crate::theme::Theme;
use crate::theme::ThemeBlock;
use crate::ui::text;

/// Draws the confirm popup into its area.
pub(crate) fn render(spec: &ConfirmSpec, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let p = &theme.palette;
    let accent = if spec.danger { p.danger } else { p.primary };
    let area = area.intersection(buf.area);
    Clear.render(area, buf);
    theme
        .block(&spec.title)
        .accent(accent)
        .seed(19)
        .render(area, buf);
    let inner = ThemeBlock::inner(area);
    if inner.height < 1 || inner.width < 2 {
        return;
    }
    let body = Rect::new(inner.x + 1, inner.y, inner.width - 2, inner.height);
    let width = usize::from(body.width);
    // The last row holds the keys, the row above it stays blank.
    let rows = usize::from(body.height.saturating_sub(2));
    for (y, line) in (body.y..).zip(spec.lines.iter().take(rows)) {
        buf.set_string(
            body.x,
            y,
            text::truncate(line, width),
            Style::new().fg(p.fg),
        );
    }
    let key = Style::new().fg(accent).add_modifier(Modifier::BOLD);
    let mute = Style::new().fg(p.fg_mute);
    Line::from(vec![
        Span::styled("y", key),
        Span::styled(" confirm · ", mute),
        Span::styled("n", Style::new().fg(p.fg).add_modifier(Modifier::BOLD)),
        Span::styled(" / ", mute),
        Span::styled("Esc", Style::new().fg(p.fg).add_modifier(Modifier::BOLD)),
        Span::styled(" cancel", mute),
    ])
    .render(Rect::new(body.x, body.bottom() - 1, body.width, 1), buf);
}
