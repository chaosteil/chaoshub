//! The key hint bar: the keys of the active context, above the status
//! bar.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::app::models::{HINT_GAP, HINT_PREFIX, KeyHintsModel};
use crate::bus::Hit;
use crate::theme::Theme;
use crate::ui::hits::HitMap;

/// Draws the key hint bar: the stencil prefix of the headers, then each
/// hint with its key in the primary color (bold) and its label in the
/// dim text color. The bar has no texture. Each hint with a chord is a
/// `Hit::Key`.
pub(crate) fn render(
    model: &KeyHintsModel,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let p = &theme.palette;
    let base = Style::new().bg(p.bg).fg(p.fg_dim);
    buf.set_style(area, base);
    if area.is_empty() {
        return;
    }
    let key = base.fg(p.primary).add_modifier(Modifier::BOLD);
    let mut spans = vec![theme.hint_prefix(HINT_PREFIX)];
    let mut x = area.x.saturating_add(width(&spans[0]));
    for (i, hint) in model.hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(HINT_GAP, base));
            x = x.saturating_add(width(&Span::raw(HINT_GAP)));
        }
        let start = spans.len();
        if !hint.key.is_empty() {
            spans.push(Span::styled(hint.key.clone(), key));
            if !hint.label.is_empty() {
                spans.push(Span::styled(" ", base));
            }
        }
        spans.push(Span::styled(hint.label.clone(), base));
        let hint_width = spans[start..]
            .iter()
            .map(width)
            .fold(0, u16::saturating_add);
        if let Some(chord) = hint.chord {
            let hint_area = Rect::new(x, area.y, hint_width, 1).intersection(area);
            hits.push(hint_area, Hit::Key(chord));
        }
        x = x.saturating_add(hint_width);
    }
    Line::from(spans).render(area, buf);
}

/// The display width of a span in cells.
fn width(span: &Span<'_>) -> u16 {
    u16::try_from(span.width()).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{KeyChord, KeyCode, KeyModifiers};
    use crate::ui::testdata;

    #[test]
    fn each_hint_with_a_chord_is_a_hit() {
        let area = Rect::new(0, 5, 60, 1);
        let mut buf = Buffer::empty(area);
        let mut hits = HitMap::default();
        render(
            &testdata::hints(),
            area,
            &mut buf,
            &testdata::pierre(),
            &mut hits,
        );
        let chord = |code| {
            Some(Hit::Key(KeyChord {
                code,
                mods: KeyModifiers::NONE,
            }))
        };
        // `▌ j/k move  Enter open  v review  m merge  / filter  ? help`
        assert_eq!(hits.at(0, 5), None, "the prefix is no hint");
        assert_eq!(hits.at(2, 5), chord(KeyCode::Char('j')));
        assert_eq!(hits.at(9, 5), chord(KeyCode::Char('j')));
        assert_eq!(hits.at(10, 5), None, "the gap is no hint");
        assert_eq!(hits.at(12, 5), chord(KeyCode::Enter));
        assert_eq!(hits.at(24, 5), chord(KeyCode::Char('v')));
        assert_eq!(hits.at(24, 4), None);
    }
}
