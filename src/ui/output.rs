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

/// The content of the output popup, borrowed from the overlay model.
pub(crate) struct Output<'a> {
    pub title: &'a str,
    pub lines: &'a [String],
    /// The index of the first visible line.
    pub scroll: usize,
}

/// Draws the output popup into its area, from the line `scroll` down.
/// When `scroll` is past the last page, the last page shows, so a popup
/// on its last line follows the tail. The whole area is a `Hit::Inside`.
pub(crate) fn render(
    output: &Output<'_>,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let p = &theme.palette;
    let lines = output.lines;
    let area = area.intersection(buf.area);
    hits.push(area, Hit::Inside);
    Clear.render(area, buf);
    theme
        .block(output.title)
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
    let skip = output.scroll.min(lines.len().saturating_sub(rows));
    let width = usize::from(body.width);
    for (y, line) in (body.y..).zip(lines.iter().skip(skip).take(rows)) {
        buf.set_string(
            body.x,
            y,
            text::truncate(line, width),
            Style::new().fg(p.fg_dim),
        );
    }
    let hint = if skip + rows < lines.len() {
        format!(
            "{} lines, lines {} to {} shown · Esc close",
            lines.len(),
            skip + 1,
            skip + rows
        )
    } else if skip > 0 {
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
        let output = Output {
            title: "output",
            lines: &lines,
            scroll: 0,
        };
        render(&output, area, &mut buf, &testdata::ansi16(), &mut hits);
        assert_eq!(hits.at(9, 5), None);
        assert_eq!(hits.at(10, 2), Some(Hit::Inside));
        assert_eq!(hits.at(30, 6), Some(Hit::Inside));
        assert_eq!(hits.at(49, 11), Some(Hit::Inside));
        assert_eq!(hits.at(50, 11), None);
    }

    /// The text of the drawn output popup with 30 lines at `scroll`.
    fn drawn(scroll: usize) -> String {
        let area = Rect::new(0, 0, 40, 8);
        let mut buf = Buffer::empty(area);
        let lines: Vec<String> = (1..=30).map(|i| format!("line {i}")).collect();
        let output = Output {
            title: "output",
            lines: &lines,
            scroll,
        };
        render(
            &output,
            area,
            &mut buf,
            &testdata::ansi16(),
            &mut HitMap::default(),
        );
        (0..8)
            .map(|y| (0..40).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn output_draws_from_the_scroll_and_stops_at_the_last_page() {
        let top = drawn(0);
        assert!(top.contains("line 1 "), "{top}");
        assert!(top.contains("30 lines, lines 1 to 5 shown"), "{top}");
        let middle = drawn(10);
        assert!(middle.contains("line 11 "), "{middle}");
        assert!(!middle.contains("line 10 "), "{middle}");
        assert_eq!(drawn(29), drawn(25), "the last page");
        assert!(drawn(29).contains("30 lines, last 5 shown"));
    }
}
