//! The text popup of a review, built on ratatui-textarea. `app` keeps the
//! text; the popup draws it with the cursor at the end, an error line
//! when the text is not valid, and the keys on the last row.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};
use ratatui_textarea::{CursorMove, TextArea, WrapMode};

use crate::bus::Hit;
use crate::theme::Theme;
use crate::theme::ThemeBlock;
use crate::ui::hits::HitMap;

/// The key hint on the last row.
const HINT: &str = "ctrl-s send · ctrl-e editor · esc cancel";

/// The placeholder of an empty text.
const PLACEHOLDER: &str = "the text of the review (optional)";

/// Draws the text popup into its area. The whole area is a
/// `Hit::Inside`.
pub(crate) fn render(
    title: &str,
    text: &str,
    error: Option<&str>,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let p = &theme.palette;
    let area = area.intersection(buf.area);
    hits.push(area, Hit::Inside);
    Clear.render(area, buf);
    theme.block(title).accent(p.warn).seed(29).render(area, buf);
    let inner = ThemeBlock::inner(area);
    let footer = if error.is_some() { 2 } else { 1 };
    if inner.height <= footer || inner.width < 2 {
        return;
    }
    let width = inner.width - 2;
    let input = Rect::new(inner.x + 1, inner.y, width, inner.height - footer);
    let mut textarea = TextArea::new(text.split('\n').map(str::to_owned).collect());
    textarea.set_wrap_mode(WrapMode::WordOrGlyph);
    textarea.set_style(Style::new().bg(p.bg).fg(p.fg));
    textarea.set_cursor_line_style(Style::new());
    textarea.set_cursor_style(Style::new().bg(p.fg).fg(p.bg));
    textarea.set_placeholder_text(PLACEHOLDER);
    textarea.set_placeholder_style(Style::new().fg(p.fg_mute).add_modifier(Modifier::ITALIC));
    textarea.move_cursor(CursorMove::Bottom);
    textarea.move_cursor(CursorMove::End);
    (&textarea).render(input, buf);
    if let Some(error) = error {
        buf.set_stringn(
            input.x,
            inner.bottom() - 2,
            error,
            usize::from(width),
            Style::new().fg(p.danger),
        );
    }
    buf.set_stringn(
        input.x,
        inner.bottom() - 1,
        HINT,
        usize::from(width),
        Style::new().fg(p.fg_mute),
    );
}
