//! The prompt popup, built on ratatui-textarea. `app` keeps the text; the
//! popup draws it in a text area with the cursor at the end.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};
use ratatui_textarea::{CursorMove, TextArea, WrapMode};

use crate::bus::{Hit, PromptSpec};
use crate::theme::Theme;
use crate::theme::ThemeBlock;
use crate::ui::hits::HitMap;

/// The key hint on the last row.
const HINT: &str = "Enter send · Esc cancel";

/// Draws the prompt popup into its area. The whole area is a
/// `Hit::Inside`.
pub(crate) fn render(
    spec: &PromptSpec,
    text: &str,
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
        .accent(p.warn)
        .seed(13)
        .render(area, buf);
    let inner = ThemeBlock::inner(area);
    if inner.height < 2 || inner.width < 2 {
        return;
    }
    let input = Rect::new(inner.x + 1, inner.y, inner.width - 2, inner.height - 1);
    let mut textarea = TextArea::new(text.split('\n').map(str::to_owned).collect());
    textarea.set_wrap_mode(WrapMode::WordOrGlyph);
    textarea.set_style(Style::new().bg(p.bg).fg(p.fg));
    textarea.set_cursor_line_style(Style::new());
    textarea.set_cursor_style(Style::new().bg(p.fg).fg(p.bg));
    textarea.set_placeholder_text(spec.placeholder.clone());
    textarea.set_placeholder_style(Style::new().fg(p.fg_mute).add_modifier(Modifier::ITALIC));
    textarea.move_cursor(CursorMove::Bottom);
    textarea.move_cursor(CursorMove::End);
    (&textarea).render(input, buf);
    buf.set_stringn(
        input.x,
        inner.bottom() - 1,
        HINT,
        usize::from(input.width),
        Style::new().fg(p.fg_mute),
    );
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
        let spec = PromptSpec {
            title: "request changes".to_owned(),
            placeholder: String::new(),
        };
        render(
            &spec,
            "text",
            area,
            &mut buf,
            &testdata::ansi16(),
            &mut hits,
        );
        assert_eq!(hits.at(9, 5), None);
        assert_eq!(hits.at(10, 2), Some(Hit::Inside));
        assert_eq!(hits.at(30, 6), Some(Hit::Inside));
        assert_eq!(hits.at(49, 11), Some(Hit::Inside));
        assert_eq!(hits.at(50, 11), None);
    }
}
