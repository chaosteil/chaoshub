//! The output popup: the lines that an action sends with
//! `ActionEvent::Output`. It follows the tail, so the last lines show.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Clear, Widget};

use crate::bus::Hit;
use crate::theme::Theme;
use crate::theme::ThemeBlock;
use crate::ui::hits::HitMap;
use crate::ui::text;

/// Draws the output popup into its area. The whole area is a
/// `Hit::Inside`.
pub(crate) fn render(
    title: &str,
    lines: &[String],
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
        .accent(p.info)
        .border(p.fg_mute)
        .seed(17)
        .render(area, buf);
    let inner = ThemeBlock::inner(area);
    if inner.height < 2 || inner.width < 2 {
        return;
    }
    let body = Rect::new(inner.x + 1, inner.y, inner.width - 2, inner.height - 1);
    let rows = usize::from(body.height);
    let skip = lines.len().saturating_sub(rows);
    let width = usize::from(body.width);
    for (y, line) in (body.y..).zip(lines.iter().skip(skip)) {
        buf.set_string(
            body.x,
            y,
            text::truncate(line, width),
            Style::new().fg(p.fg_dim),
        );
    }
    let hint = if skip > 0 {
        format!("{} lines, last {rows} shown · Esc close", lines.len())
    } else {
        format!("{} lines · Esc close", lines.len())
    };
    buf.set_stringn(
        body.x,
        inner.bottom() - 1,
        hint,
        width,
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
        let lines = vec!["one".to_owned(), "two".to_owned()];
        render(
            "output",
            &lines,
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
