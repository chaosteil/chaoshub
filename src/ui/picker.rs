//! The picker popup: a list of items with a cursor, for example the
//! author filter. The list scrolls so the cursor stays visible.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::bus::Hit;
use crate::theme::Theme;
use crate::theme::ThemeBlock;
use crate::theme::punk;
use crate::ui::hits::HitMap;
use crate::ui::text;

/// Draws the picker popup into its area. The whole area is a
/// `Hit::Inside`, and each drawn item is a `Hit::Item` with its index.
pub(crate) fn render(
    title: &str,
    items: &[String],
    cursor: usize,
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
        .block(title)
        .accent(p.primary)
        .seed(23)
        .render(area, buf);
    let inner = ThemeBlock::inner(area);
    if inner.height == 0 || inner.width < 2 {
        return;
    }
    let rows = usize::from(inner.height);
    let offset = cursor.saturating_sub(rows - 1);
    let selection = theme.selected_row();
    let tail = if selection.tail.is_some() { 2 } else { 0 };
    let width = usize::from(inner.width.saturating_sub(2 + tail + selection.shift));
    for (y, (index, item)) in (inner.y..).zip(items.iter().enumerate().skip(offset).take(rows)) {
        let line = Rect::new(inner.x, y, inner.width, 1);
        hits.push(line, Hit::Item(index));
        if index == cursor {
            let style = Style::new().fg(p.fg).patch(selection.patch);
            buf.set_style(line, style);
            let marker = style.fg(selection.marker_fg);
            let marker = selection.marker_bg.map_or(marker, |bg| marker.bg(bg));
            buf.set_string(inner.x, y, selection.marker, marker);
            if let Some([(first, first_style), (second, second_style)]) = selection.tail {
                buf.set_string(line.right() - 2, y, first, first_style);
                buf.set_string(line.right() - 1, y, second, second_style);
            }
            buf.set_string(
                inner.x + 2,
                y,
                text::truncate(item, width),
                style.add_modifier(Modifier::BOLD),
            );
            // The band jumps out: its text moves into the free cells.
            let content = Rect::new(inner.x + 1, y, inner.width.saturating_sub(1 + tail), 1);
            punk::shift_right(buf, content, selection.shift, style);
        } else {
            plain_item(item, line, buf, theme);
        }
    }
}

/// Draws an item without the cursor on `line`, a row of the picker. The
/// transitions use it for the row under a band wipe.
pub(crate) fn plain_item(item: &str, line: Rect, buf: &mut Buffer, theme: &Theme) {
    let p = &theme.palette;
    let selection = theme.selected_row();
    let tail = if selection.tail.is_some() { 2 } else { 0 };
    let width = usize::from(line.width.saturating_sub(2 + tail + selection.shift));
    buf.set_string(
        line.x + 2,
        line.y,
        text::truncate(item, width),
        Style::new().fg(p.fg_dim),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testdata;

    #[test]
    fn hits_of_the_picker() {
        let area = Rect::new(10, 2, 30, 7);
        let items: Vec<String> = (0..10).map(|i| format!("colleague-{i:02}")).collect();
        let mut buf = Buffer::empty(Rect::new(0, 0, 50, 12));
        let mut hits = HitMap::default();
        render(
            "author",
            &items,
            7,
            area,
            &mut buf,
            &testdata::ansi16(),
            &mut hits,
        );
        assert_eq!(hits.at(9, 4), None, "left of the picker");
        assert_eq!(hits.at(10, 4), Some(Hit::Inside), "the border");
        // Five rows show items 3 to 7, so the cursor stays visible.
        assert_eq!(hits.at(12, 3), Some(Hit::Item(3)));
        assert_eq!(hits.at(38, 7), Some(Hit::Item(7)));
        assert_eq!(hits.at(12, 8), Some(Hit::Inside), "the bottom border");
    }
}
