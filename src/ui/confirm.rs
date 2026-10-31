//! The confirm popup: a title, the lines of the action, and the keys. A
//! dangerous action (merge, close) gets the danger color.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};

use crate::bus::{ConfirmSpec, Hit};
use crate::theme::Theme;
use crate::theme::ThemeBlock;
use crate::ui::hits::HitMap;
use crate::ui::text;

/// Draws the confirm popup into its area. The whole area is a
/// `Hit::Inside`.
pub(crate) fn render(
    spec: &ConfirmSpec,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let p = &theme.palette;
    let accent = if spec.danger { p.danger } else { p.primary };
    let area = area.intersection(buf.area);
    hits.push(area, Hit::Inside);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testdata;

    #[test]
    fn the_area_is_inside() {
        let area = Rect::new(10, 2, 40, 10);
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 15));
        let mut hits = HitMap::default();
        let overlay = testdata::confirm_overlay(true);
        let crate::app::models::OverlayContent::Confirm(spec) = &overlay.content else {
            unreachable!()
        };
        render(spec, area, &mut buf, &testdata::ansi16(), &mut hits);
        assert_eq!(hits.at(9, 5), None);
        assert_eq!(hits.at(10, 2), Some(Hit::Inside));
        assert_eq!(hits.at(30, 6), Some(Hit::Inside));
        assert_eq!(hits.at(49, 11), Some(Hit::Inside));
        assert_eq!(hits.at(50, 11), None);
    }
}
