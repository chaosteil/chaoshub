//! The filter line: `/` and the filter text, with a cursor while the user
//! types.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::app::models::FilterModel;
use crate::theme::Theme;

/// Draws the filter line into a one-row area.
pub(crate) fn render(model: &FilterModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let p = &theme.palette;
    let base = Style::new().bg(p.surface).fg(p.fg);
    buf.set_style(area, base);
    let mut spans = vec![
        Span::styled(" / ", base.fg(p.primary).add_modifier(Modifier::BOLD)),
        Span::styled(model.text.clone(), base),
    ];
    if model.editing {
        spans.push(Span::styled("▏", base.fg(p.primary)));
    } else {
        spans.push(Span::styled("  / to edit", base.fg(p.fg_mute)));
    }
    Line::from(spans).render(area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testdata;

    fn row(model: &FilterModel, width: u16) -> (String, Buffer) {
        let area = Rect::new(0, 0, width, 1);
        let mut buf = Buffer::empty(area);
        render(model, area, &mut buf, &testdata::pierre());
        let text = (0..width).map(|x| buf[(x, 0)].symbol()).collect();
        (text, buf)
    }

    #[test]
    fn editing_shows_a_cursor() {
        let model = FilterModel {
            text: "pane".into(),
            editing: true,
        };
        let (text, buf) = row(&model, 20);
        assert_eq!(text, " / pane▏            ");
        assert_eq!(buf[(1, 0)].fg, testdata::pierre().palette.primary);
        assert_eq!(buf[(10, 0)].bg, testdata::pierre().palette.surface);
    }

    #[test]
    fn a_kept_filter_shows_the_hint() {
        let model = FilterModel {
            text: "pane".into(),
            editing: false,
        };
        let (text, _) = row(&model, 30);
        assert_eq!(text, " / pane  / to edit            ");
    }
}
