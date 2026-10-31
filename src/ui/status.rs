//! The status bar: the sync state on the left, the last message in the
//! center, the help hint on the right.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Widget;

use crate::app::models::StatusModel;
use crate::theme::Theme;
use crate::ui::style;

/// The space at each end of the bar.
const PAD: u16 = 1;
/// The minimum space between the message and its neighbors.
const SPACE: u16 = 2;

/// Draws the status bar.
pub(crate) fn render(model: &StatusModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let palette = &theme.palette;
    buf.set_style(area, Style::new().bg(palette.surface).fg(palette.fg));
    if area.width <= 2 * PAD || area.height == 0 {
        return;
    }
    let inner = Rect::new(area.x + PAD, area.y, area.width - 2 * PAD, 1);
    let left = style::line(&model.left, palette);
    let right = style::line(&model.right, palette);
    let right_width = u16::try_from(right.width())
        .unwrap_or(u16::MAX)
        .min(inner.width);
    let right_x = inner.right() - right_width;
    right.render(Rect::new(right_x, inner.y, right_width, 1), buf);
    let left_width = u16::try_from(left.width())
        .unwrap_or(u16::MAX)
        .min(right_x - inner.x);
    left.render(Rect::new(inner.x, inner.y, left_width, 1), buf);
    let Some(center) = &model.center else {
        return;
    };
    // The message sits in the middle of the bar, or as close to it as the
    // left and right parts permit. A long message is cut.
    let lo = inner.x + left_width + if left_width > 0 { SPACE } else { 0 };
    let hi = right_x.saturating_sub(if right_width > 0 { SPACE } else { 0 });
    if hi <= lo {
        return;
    }
    let free = hi - lo;
    let text = crate::ui::text::truncate(&center.text, usize::from(free));
    let width = u16::try_from(crate::ui::text::width(&text)).unwrap_or(free);
    let middle = (area.x + area.width / 2).saturating_sub(width / 2);
    let x = middle.clamp(lo, hi - width);
    buf.set_string(
        x,
        inner.y,
        text,
        style::cell_style(center.style, palette).bg(palette.surface),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::models::CellStyle;
    use crate::ui::testdata::{self, cell};

    fn draw(model: &StatusModel, width: u16, theme: &Theme) -> Buffer {
        let area = Rect::new(0, 0, width, 1);
        let mut buf = Buffer::empty(area);
        render(model, area, &mut buf, theme);
        buf
    }

    fn row(buf: &Buffer) -> String {
        (0..buf.area.width).map(|x| buf[(x, 0)].symbol()).collect()
    }

    #[test]
    fn status_bar_100_pierre() {
        insta::assert_snapshot!(row(&draw(&testdata::status(), 100, &testdata::pierre())));
    }

    #[test]
    fn status_bar_160_pierre() {
        insta::assert_snapshot!(row(&draw(&testdata::status(), 160, &testdata::pierre())));
    }

    #[test]
    fn status_bar_100_ansi16() {
        insta::assert_snapshot!(row(&draw(&testdata::status(), 100, &testdata::ansi16())));
    }

    #[test]
    fn status_bar_160_ansi16() {
        insta::assert_snapshot!(row(&draw(&testdata::status(), 160, &testdata::ansi16())));
    }

    #[test]
    fn cache_age_without_message() {
        let model = StatusModel {
            left: vec![
                cell("[MINE]", CellStyle::Primary),
                cell(" 3 repos · cache from 42s ago", CellStyle::Dim),
            ],
            center: None,
            right: vec![cell("? help", CellStyle::Dim)],
        };
        insta::assert_snapshot!(row(&draw(&model, 100, &testdata::pierre())));
    }

    #[test]
    fn long_message_is_cut() {
        let mut model = testdata::status();
        model.center = Some(cell(
            "sync failed: the GitHub API answered 502 Bad Gateway three times in a row",
            CellStyle::Danger,
        ));
        let buf = draw(&model, 100, &testdata::pierre());
        insta::assert_snapshot!(row(&buf));
        assert!(row(&buf).contains("Ba…  ? help"));
    }

    #[test]
    fn errors_use_the_danger_color() {
        let mut model = testdata::status();
        model.center = Some(cell("rate limited until 14:05", CellStyle::Danger));
        let theme = testdata::pierre();
        let buf = draw(&model, 100, &theme);
        let x = u16::try_from(row(&buf).find("rate").unwrap()).unwrap();
        assert_eq!(buf[(x, 0)].fg, theme.palette.danger);
        assert_eq!(buf[(x, 0)].bg, theme.palette.surface);
    }
}
