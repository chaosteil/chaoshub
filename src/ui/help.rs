//! The help overlay: the key bindings in sections, at the right edge of
//! the screen. The transitions slide it in from the right.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::app::models::HelpModel;
use crate::bus::Hit;
use crate::theme::Theme;
use crate::theme::ThemeBlock;
use crate::ui::hits::HitMap;
use crate::ui::text;

/// The minimum width of the key column.
const KEY_MIN: usize = 6;

/// Draws the help overlay into its area. The whole area is a
/// `Hit::Inside`.
pub(crate) fn render(
    model: &HelpModel,
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
        .block("help")
        .accent(p.info)
        .seed(11)
        .render(area, buf);
    let inner = ThemeBlock::inner(area);
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
        theme
            .header(&section.title, p.primary)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testdata;

    #[test]
    fn the_area_is_inside() {
        let area = Rect::new(10, 2, 40, 10);
        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 15));
        let mut hits = HitMap::default();
        let help = testdata::help_overlay();
        let crate::app::models::OverlayContent::Help(model) = &help.content else {
            unreachable!()
        };
        render(model, area, &mut buf, &testdata::ansi16(), &mut hits);
        assert_eq!(hits.at(9, 5), None);
        assert_eq!(hits.at(10, 2), Some(Hit::Inside));
        assert_eq!(hits.at(30, 6), Some(Hit::Inside));
        assert_eq!(hits.at(49, 11), Some(Hit::Inside));
        assert_eq!(hits.at(50, 11), None);
    }
}
