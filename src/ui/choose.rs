//! The choice popup: a title, the lines, and one key per option. `n` and
//! `Esc` cancel.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};

use crate::bus::{ChooseSpec, Hit};
use crate::theme::Theme;
use crate::theme::ThemeBlock;
use crate::ui::hits::HitMap;
use crate::ui::text;

/// Draws the choice popup into its area. The whole area is a
/// `Hit::Inside`.
pub(crate) fn render(
    spec: &ChooseSpec,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let p = &theme.palette;
    let area = area.intersection(buf.area);
    hits.push(area, Hit::Inside);
    Clear.render(area, buf);
    theme
        .block(&spec.title)
        .accent(p.primary)
        .seed(23)
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
    let key = Style::new().fg(p.primary).add_modifier(Modifier::BOLD);
    let mute = Style::new().fg(p.fg_mute);
    let mut spans = Vec::new();
    for (c, label) in &spec.options {
        spans.push(Span::styled(c.to_string(), key));
        spans.push(Span::styled(format!(" {label} · "), mute));
    }
    let bold = Style::new().fg(p.fg).add_modifier(Modifier::BOLD);
    spans.push(Span::styled("n", bold));
    spans.push(Span::styled(" / ", mute));
    spans.push(Span::styled("Esc", bold));
    spans.push(Span::styled(" cancel", mute));
    Line::from(spans).render(Rect::new(body.x, body.bottom() - 1, body.width, 1), buf);
}
