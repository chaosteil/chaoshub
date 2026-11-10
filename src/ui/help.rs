//! The help overlay: the key bindings and the list symbols in sections,
//! at the right edge of the screen. The transitions slide it in from the
//! right.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::app::models::{Cell, HelpModel};
use crate::bus::Hit;
use crate::theme::Theme;
use crate::theme::ThemeBlock;
use crate::ui::hits::HitMap;
use crate::ui::style::cell_style;
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
    /// A styled symbol and its label, with the width of the key column of
    /// the section.
    Symbol(&'a Cell, &'a str, usize),
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
            .chain(
                section
                    .symbols
                    .iter()
                    .map(|(cell, _)| text::width(&cell.text)),
            )
            .max()
            .unwrap_or(0)
            .max(KEY_MIN);
        lines.extend(
            section
                .entries
                .iter()
                .map(|(key, label)| HelpLine::Entry(key, label, key_width)),
        );
        lines.extend(
            section
                .symbols
                .iter()
                .map(|(cell, label)| HelpLine::Symbol(cell, label, key_width)),
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
    let lines = help_lines(model);
    let rows = usize::from(inner.height);
    let skip = scroll.min(lines.len().saturating_sub(rows));
    for (y, line) in (inner.y..).zip(lines.into_iter().skip(skip).take(rows)) {
        let row = Rect::new(inner.x, y, inner.width, 1);
        match line {
            HelpLine::Blank => {}
            HelpLine::Title(title) => theme.header(title, p.primary).render(row, buf),
            HelpLine::Entry(key, label, key_width) => {
                let key_style = Style::new().fg(p.primary).add_modifier(Modifier::BOLD);
                entry(key, key_style, label, key_width, row, buf, theme);
            }
            HelpLine::Symbol(cell, label, key_width) => {
                let key_style = cell_style(cell.style, p);
                entry(&cell.text, key_style, label, key_width, row, buf, theme);
            }
        }
    }
}

/// Draws one entry into the one-line `row`: the key in `key_style` in the
/// key column of `key_width` cells, then the label. A label that does not
/// fit ends with `…`.
fn entry(
    key: &str,
    key_style: Style,
    label: &str,
    key_width: usize,
    row: Rect,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let key_text = text::fit(
        key,
        key_width.min(usize::from(row.width)),
        crate::app::models::Align::Left,
    );
    buf.set_string(row.x, row.y, &key_text, key_style);
    let label_x = row.x + u16::try_from(key_width + 1).unwrap_or(row.width);
    if label_x < row.right() {
        let room = usize::from(row.right() - label_x);
        buf.set_string(
            label_x,
            row.y,
            text::truncate(label, room),
            Style::new().fg(theme.palette.fg),
        );
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
        assert_eq!(help.line_count(), 18);
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
        assert_eq!(draw(17), draw(13), "the last page");
        assert!(draw(17).contains("re-run failed checks"));
    }

    #[test]
    fn symbols_have_the_style_of_their_cell() {
        let help = testdata::help_overlay();
        let crate::app::models::OverlayContent::Help { help, .. } = &help.content else {
            unreachable!()
        };
        for theme in [testdata::ansi16(), testdata::pierre(), testdata::punk()] {
            let area = Rect::new(0, 0, 40, 30);
            let mut buf = Buffer::empty(area);
            render(help, 0, area, &mut buf, &theme, &mut HitMap::default());
            let p = &theme.palette;
            let at = |symbol: &str| {
                (0..30)
                    .flat_map(|y| (0..40).map(move |x| (x, y)))
                    .find(|&(x, y)| buf[(x, y)].symbol() == symbol)
                    .map(|(x, y)| &buf[(x, y)])
                    .unwrap()
            };
            assert_eq!(at("✓").fg, p.success);
            assert_eq!(at("◆").fg, p.warn);
            assert!(at("◆").modifier.contains(Modifier::BOLD));
            assert_eq!(at("a").fg, p.fg, "the label");
        }
    }
}
