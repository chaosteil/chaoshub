//! The key hint bar: the keys of the active context, above the status
//! bar.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::app::models::{HINT_GAP, HINT_PREFIX, KeyHintsModel};
use crate::theme::Theme;

/// Draws the key hint bar: the stencil prefix of the headers, then each
/// hint with its key in the primary color (bold) and its label in the
/// dim text color. The bar has no texture.
pub(crate) fn render(model: &KeyHintsModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let p = &theme.palette;
    let base = Style::new().bg(p.bg).fg(p.fg_dim);
    buf.set_style(area, base);
    if area.is_empty() {
        return;
    }
    let key = base.fg(p.primary).add_modifier(Modifier::BOLD);
    let mut spans = vec![Span::styled(HINT_PREFIX, base.fg(p.primary))];
    for (i, hint) in model.hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(HINT_GAP, base));
        }
        if !hint.key.is_empty() {
            spans.push(Span::styled(hint.key.clone(), key));
            if !hint.label.is_empty() {
                spans.push(Span::styled(" ", base));
            }
        }
        spans.push(Span::styled(hint.label.clone(), base));
    }
    Line::from(spans).render(area, buf);
}
