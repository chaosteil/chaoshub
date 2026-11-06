//! The status bar: the sync state on the left, the last message in the
//! center, the help hint on the right.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use crate::app::models::{CellStyle, StatusModel};
use crate::theme::Theme;
use crate::ui::style;

/// The space at each end of the bar.
const PAD: u16 = 1;
/// The minimum space between the message and its neighbors.
const SPACE: u16 = 2;

/// The places of the parts of the bar.
struct Parts {
    /// The bar without the space at each end.
    inner: Rect,
    /// The width of the left part, without the band end.
    left_width: u16,
    /// The cells of the band end after the left part.
    tail: u16,
    right_x: u16,
    right_width: u16,
}

/// Where the parts of the bar go. `None` when the bar is too small.
fn parts(model: &StatusModel, area: Rect, theme: &Theme) -> Option<Parts> {
    if area.width <= 2 * PAD || area.height == 0 {
        return None;
    }
    let palette = &theme.palette;
    let inner = Rect::new(area.x + PAD, area.y, area.width - 2 * PAD, 1);
    let right_width = u16::try_from(style::line(&model.right, palette).width())
        .unwrap_or(u16::MAX)
        .min(inner.width);
    let right_x = inner.right() - right_width;
    // The band takes one cell after the text and one for its end.
    let tail = if theme.status_band().end.is_some() {
        2
    } else {
        0
    };
    let left_width = u16::try_from(style::line(&model.left, palette).width())
        .unwrap_or(u16::MAX)
        .min((right_x - inner.x).saturating_sub(tail));
    Some(Parts {
        inner,
        left_width,
        tail,
        right_x,
        right_width,
    })
}

/// The place of the message: the message area between the left and the
/// right part, and the x and the text of the message in that area. The
/// message sits in the middle of the bar, or as close to it as the left
/// and right parts permit. A long message is cut. `None` without a
/// message or without room.
pub(crate) struct MessagePlace {
    pub area: Rect,
    pub x: u16,
    pub text: String,
}

/// The place of the message of `model` in the bar `area`.
pub(crate) fn message_place(
    model: &StatusModel,
    area: Rect,
    theme: &Theme,
) -> Option<MessagePlace> {
    let center = model.center.as_ref()?;
    let parts = parts(model, area, theme)?;
    let left_width = parts.left_width + parts.tail;
    let lo = parts.inner.x + left_width + if left_width > 0 { SPACE } else { 0 };
    let hi = parts
        .right_x
        .saturating_sub(if parts.right_width > 0 { SPACE } else { 0 });
    if hi <= lo {
        return None;
    }
    let free = hi - lo;
    let text = crate::ui::text::truncate(&center.text, usize::from(free));
    let width = u16::try_from(crate::ui::text::width(&text)).unwrap_or(free);
    let middle = (area.x + area.width / 2).saturating_sub(width / 2);
    Some(MessagePlace {
        area: Rect::new(lo, parts.inner.y, free, 1),
        x: middle.clamp(lo, hi - width),
        text,
    })
}

/// Draws the status bar. The theme picks the bar style, an optional band
/// behind the left part with a diagonal end, and the message color.
pub(crate) fn render(model: &StatusModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let palette = &theme.palette;
    let band = theme.status_band();
    buf.set_style(area, band.bar);
    let Some(parts) = parts(model, area, theme) else {
        return;
    };
    let Parts {
        inner,
        left_width,
        tail,
        right_x,
        right_width,
    } = parts;
    let mut left = style::line(&model.left, palette);
    if let Some(style) = band.left {
        for span in &mut left.spans {
            span.style = style;
        }
    }
    let right = style::line(&model.right, palette);
    right.render(Rect::new(right_x, inner.y, right_width, 1), buf);
    if let Some(style) = band.left {
        let width = inner.x - area.x + left_width + tail.saturating_sub(1);
        buf.set_style(Rect::new(area.x, area.y, width, 1), style);
    }
    left.render(Rect::new(inner.x, inner.y, left_width, 1), buf);
    if let Some((glyph, style)) = band.end {
        buf.set_string(inner.x + left_width + 1, inner.y, glyph, style);
    }
    let (Some(center), Some(MessagePlace { x, text, .. })) =
        (&model.center, message_place(model, area, theme))
    else {
        return;
    };
    let mut message = band.bar.patch(style::cell_style(center.style, palette));
    // A message with a state keeps its state color.
    if !matches!(center.style, CellStyle::Danger | CellStyle::Warn)
        && let Some(fg) = band.message_fg
    {
        message = message.fg(fg);
    }
    buf.set_string(x, inner.y, text, message);
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn punk_status_has_a_red_band_on_the_left() {
        let theme = testdata::punk();
        let p = theme.palette;
        let buf = draw(&testdata::status(), 100, &theme);
        let text = row(&buf);
        assert!(text.starts_with(" [MINE] 3 repos"), "{text}");
        let end = text.chars().position(|c| c == '◤').unwrap();
        for x in 0..u16::try_from(end).unwrap() {
            assert_eq!(buf[(x, 0)].bg, p.primary, "x {x}");
            assert_eq!(buf[(x, 0)].fg, p.bg, "x {x}");
        }
        let end = u16::try_from(end).unwrap();
        assert_eq!(buf[(end, 0)].fg, p.primary);
        assert_eq!(buf[(end, 0)].bg, p.bg);
        let x = u16::try_from(text.chars().position(|c| c == 'm').unwrap()).unwrap();
        assert_eq!(
            text.chars()
                .skip(usize::from(x))
                .take(6)
                .collect::<String>(),
            "merged"
        );
        assert_eq!(buf[(x, 0)].fg, p.fg);
        assert_eq!(buf[(x, 0)].bg, p.bg);
        assert!(text.ends_with("? help "), "{text}");
    }

    #[test]
    fn punk_errors_keep_the_danger_color() {
        let mut model = testdata::status();
        model.center = Some(cell("rate limited until 14:05", CellStyle::Danger));
        let theme = testdata::punk();
        let buf = draw(&model, 100, &theme);
        let x = u16::try_from(row(&buf).find("rate").unwrap()).unwrap();
        assert_eq!(buf[(x, 0)].fg, theme.palette.danger);
    }
}
