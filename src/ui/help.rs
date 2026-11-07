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

/// One line of the help overlay.
enum HelpLine<'a> {
    /// The space between two sections.
    Blank,
    /// The title of a section.
    Title(&'a str),
    /// A key and its label, with the width of the key column of the
    /// section.
    Entry(&'a str, &'a str, usize),
}

/// The lines of the help overlay, in draw order. There are
/// `HelpModel::line_count` lines.
fn help_lines(model: &HelpModel) -> Vec<HelpLine<'_>> {
    let mut lines = Vec::with_capacity(model.line_count());
    for (i, section) in model.sections.iter().enumerate() {
        if i > 0 {
            lines.push(HelpLine::Blank);
        }
        lines.push(HelpLine::Title(&section.title));
        let key_width = section
            .entries
            .iter()
            .map(|(key, _)| text::width(key))
            .max()
            .unwrap_or(0)
            .max(KEY_MIN);
        lines.extend(
            section
                .entries
                .iter()
                .map(|(key, label)| HelpLine::Entry(key, label, key_width)),
        );
    }
    lines
}

/// Draws the help overlay into its area, from the line `scroll` down.
/// When `scroll` is past the last page, the last page shows. The whole
/// area is a `Hit::Inside`.
pub(crate) fn render(
    model: &HelpModel,
    scroll: usize,
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
    hits.push_text(inner);
    let width = usize::from(inner.width);
    let lines = help_lines(model);
    let rows = usize::from(inner.height);
    let skip = scroll.min(lines.len().saturating_sub(rows));
    for (y, line) in (inner.y..).zip(lines.into_iter().skip(skip).take(rows)) {
        match line {
            HelpLine::Blank => {}
            HelpLine::Title(title) => theme
                .header(title, p.primary)
                .render(Rect::new(inner.x, y, inner.width, 1), buf),
            HelpLine::Entry(key, label, key_width) => {
                let key_text =
                    text::fit(key, key_width.min(width), crate::app::models::Align::Left);
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
            }
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
        let crate::app::models::OverlayContent::Help { help, .. } = &help.content else {
            unreachable!()
        };
        render(help, 0, area, &mut buf, &testdata::ansi16(), &mut hits);
        assert_eq!(hits.at(9, 5), None);
        assert_eq!(hits.at(10, 2), Some(Hit::Inside));
        assert_eq!(hits.at(30, 6), Some(Hit::Inside));
        assert_eq!(hits.at(49, 11), Some(Hit::Inside));
        assert_eq!(hits.at(50, 11), None);
    }

    #[test]
    fn help_draws_from_the_scroll() {
        let help = testdata::help_overlay();
        let crate::app::models::OverlayContent::Help { help, .. } = &help.content else {
            unreachable!()
        };
        assert_eq!(help.line_count(), 13);
        let draw = |scroll| {
            let area = Rect::new(0, 0, 40, 8);
            let mut buf = Buffer::empty(area);
            render(
                help,
                scroll,
                area,
                &mut buf,
                &testdata::ansi16(),
                &mut HitMap::default(),
            );
            (0..8)
                .map(|y| (0..40).map(|x| buf[(x, y)].symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(draw(0).contains("move down / up"));
        let scrolled = draw(2);
        assert!(!scrolled.contains("move down / up"), "{scrolled}");
        assert!(scrolled.contains("first / last row"), "{scrolled}");
        assert_eq!(draw(12), draw(8), "the last page");
        assert!(draw(12).contains("re-run failed checks"));
    }
}
