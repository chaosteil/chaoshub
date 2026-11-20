//! The toast: a short card at the bottom right, above the key hint bar.
//! It says what a key did when the screen does not show it at once, for
//! example `copied url of #142`. It takes no keys and no clicks.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Clear, Widget};

use crate::app::layout::Areas;
use crate::app::models::ToastModel;
use crate::theme::{Skin, Theme, ThemeBlock};
use crate::ui::{rect, text};

/// The height of the card: the frame and one row of text.
const HEIGHT: u16 = 3;
/// The cells of the card around the text: the frame and one space on
/// each side.
const FRAME: u16 = 4;
/// The columns right of the card, and the rows below it. The punk drop
/// shadow takes the first column and the first row. The second column
/// and row stay free, so the card does not cover the frame of the popup
/// below it.
const MARGIN: u16 = 2;

/// The area of the card with `text` on the screen of `areas`: at the
/// bottom right of the rows between the tab bar and the key hint bar
/// (or the status bar, when the key hints are off). A long text is cut
/// to the width of the screen. `None` when the screen is too small.
pub(crate) fn area(text: &str, areas: &Areas) -> Option<Rect> {
    let screen = rect(areas.list);
    let bars = if areas.hints.height > 0 {
        areas.hints.y
    } else {
        areas.status.y
    };
    let bottom = bars.checked_sub(MARGIN)?;
    let top = bottom.checked_sub(HEIGHT)?;
    // The card stays below the tab bar.
    if top < screen.y {
        return None;
    }
    let room = screen.width.checked_sub(2 * MARGIN)?;
    if room <= FRAME {
        return None;
    }
    let width = u16::try_from(text::width(text))
        .unwrap_or(u16::MAX)
        .saturating_add(FRAME)
        .min(room);
    let x = screen.right() - MARGIN - width;
    Some(Rect::new(x, top, width, HEIGHT))
}

/// The area of the card and of its drop shadow: the punk skin draws the
/// shadow one column right of the card and one row below it.
pub(crate) fn effect_area(card: Rect, theme: &Theme) -> Rect {
    if theme.skin == Skin::Punk {
        Rect::new(card.x, card.y, card.width + 1, card.height + 1)
    } else {
        card
    }
}

/// Draws the toast: a card with the frame of the theme in the primary
/// color, and the text in bold.
pub(crate) fn render(model: &ToastModel, areas: &Areas, buf: &mut Buffer, theme: &Theme) {
    let Some(area) = area(&model.text, areas) else {
        return;
    };
    let area = area.intersection(buf.area);
    let p = &theme.palette;
    crate::ui::cut_wide(buf, area);
    Clear.render(area, buf);
    ThemeBlock::new(theme, "")
        .accent(p.primary)
        .seed(41)
        .render(area, buf);
    let inner = ThemeBlock::inner(area);
    let width = usize::from(inner.width.saturating_sub(2));
    if width == 0 || inner.height == 0 {
        return;
    }
    let style = Style::new().fg(p.fg).bg(p.bg).add_modifier(Modifier::BOLD);
    buf.set_string(
        inner.x + 1,
        inner.y,
        text::truncate(&model.text, width),
        style,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::models::FrameModel;
    use crate::ui::testdata;

    /// The list with the key hints and a toast.
    fn toast(text: &str) -> FrameModel {
        FrameModel {
            toast: Some(ToastModel { text: text.into() }),
            hints: Some(testdata::hints()),
            ..testdata::frame()
        }
    }

    fn draw(model: &FrameModel, width: u16, height: u16, theme: &Theme) -> String {
        crate::ui::tests::draw(model, width, height, theme)
    }

    #[test]
    fn the_card_sits_above_the_key_hint_bar_at_the_right() {
        let areas = testdata::areas(100, 30);
        let card = area("copied url of #142", &areas).unwrap();
        assert_eq!(card.width, 18 + FRAME);
        assert_eq!(card.right(), 100 - MARGIN);
        assert_eq!(card.bottom(), areas.hints.y - MARGIN);
        assert_eq!(card.height, HEIGHT);
    }

    #[test]
    fn without_key_hints_the_card_sits_above_the_status_bar() {
        let size = crate::bus::Size {
            width: 100,
            height: 30,
        };
        let areas = crate::app::layout::layout_with(size, false);
        let card = area("copied url of #142", &areas).unwrap();
        assert_eq!(card.bottom(), areas.status.y - MARGIN);
    }

    #[test]
    fn a_long_text_is_cut_to_the_screen() {
        let areas = testdata::areas(30, 30);
        let card = area(&"x".repeat(80), &areas).unwrap();
        assert_eq!(card.width, 30 - 2 * MARGIN);
        assert_eq!(card.x, MARGIN);
        let screen = draw(&toast(&"x".repeat(80)), 30, 30, &testdata::punk());
        assert!(screen.contains('…'), "{screen}");
    }

    #[test]
    fn a_small_screen_has_no_toast() {
        assert_eq!(area("copied", &testdata::areas(100, 5)), None);
        assert_eq!(area("copied", &testdata::areas(5, 30)), None);
        // The draw does not panic.
        draw(&toast("copied"), 100, 5, &testdata::punk());
        draw(&toast("copied"), 5, 30, &testdata::punk());
    }

    #[test]
    fn punk_toast_has_the_frame_of_the_popups() {
        let theme = testdata::punk();
        let p = theme.palette;
        let areas = testdata::areas(100, 30);
        let card = area("copied url of #142", &areas).unwrap();
        let model = toast("copied url of #142");
        let mut terminal = testdata::terminal(100, 30);
        terminal
            .draw(|frame| {
                crate::ui::view(
                    frame,
                    &model,
                    &areas,
                    &theme,
                    &mut crate::ui::hits::HitMap::default(),
                );
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        assert_eq!(buf[(card.x, card.y)].symbol(), "◢");
        assert_eq!(buf[(card.right() - 1, card.y)].symbol(), "◣");
        assert_eq!(buf[(card.x, card.bottom() - 1)].symbol(), "◥");
        assert_eq!(buf[(card.right() - 1, card.bottom() - 1)].symbol(), "◤");
        assert_eq!(buf[(card.x, card.y + 1)].symbol(), "▌");
        assert_eq!(buf[(card.right() - 1, card.y + 1)].symbol(), "▐");
        assert_eq!(buf[(card.x, card.y)].fg, p.primary);
        let text = buf[(card.x + 2, card.y + 1)].clone();
        assert_eq!(text.symbol(), "c");
        assert_eq!(text.fg, p.fg);
        assert!(text.modifier.contains(Modifier::BOLD));
        // The drop shadow ends one row above the key hint bar.
        assert_eq!(buf[(card.right(), card.bottom())].symbol(), "◤");
        assert_eq!(card.bottom(), areas.hints.y - 2);
    }

    #[test]
    fn the_toast_takes_no_clicks() {
        let model = toast("copied url of #142");
        let areas = testdata::areas(100, 30);
        let card = area("copied url of #142", &areas).unwrap();
        let mut hits = crate::ui::hits::HitMap::default();
        let mut terminal = testdata::terminal(100, 30);
        terminal
            .draw(|frame| {
                crate::ui::view(frame, &model, &areas, &testdata::punk(), &mut hits);
            })
            .unwrap();
        let mut plain = crate::ui::hits::HitMap::default();
        let without = FrameModel {
            toast: None,
            ..model.clone()
        };
        terminal
            .draw(|frame| {
                crate::ui::view(frame, &without, &areas, &testdata::punk(), &mut plain);
            })
            .unwrap();
        for (x, y) in [(card.x + 2, card.y + 1), (card.x, card.y)] {
            assert_eq!(hits.at(x, y), plain.at(x, y), "{x} {y}");
        }
    }

    #[test]
    fn punk_toast_on_16_colors_uses_the_ansi_palette() {
        let theme = testdata::punk_ansi16();
        let p = theme.palette;
        let areas = testdata::areas(100, 30);
        let card = area("copied url of #142", &areas).unwrap();
        let mut terminal = testdata::terminal(100, 30);
        terminal
            .draw(|frame| {
                crate::ui::view(
                    frame,
                    &toast("copied url of #142"),
                    &areas,
                    &theme,
                    &mut crate::ui::hits::HitMap::default(),
                );
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        assert_eq!(buf[(card.x, card.y)].fg, p.primary);
        assert_eq!(buf[(card.x + 2, card.y + 1)].fg, p.fg);
        assert_eq!(buf[(card.x + 2, card.y + 1)].bg, p.bg);
    }

    #[test]
    fn toast_100_punk() {
        insta::assert_snapshot!(draw(
            &toast("copied url of #142"),
            100,
            30,
            &testdata::punk()
        ));
    }

    #[test]
    fn toast_160_punk() {
        insta::assert_snapshot!(draw(
            &toast("copied 3 urls of stack #143"),
            160,
            40,
            &testdata::punk()
        ));
    }

    #[test]
    fn toast_100_ansi16() {
        insta::assert_snapshot!(draw(
            &toast("copied url of #142"),
            100,
            30,
            &testdata::ansi16()
        ));
    }

    #[test]
    fn toast_160_pierre() {
        insta::assert_snapshot!(draw(
            &toast("opened #142 in browser"),
            160,
            40,
            &testdata::pierre()
        ));
    }

    #[test]
    fn toast_over_the_popup_100_punk() {
        let model = FrameModel {
            overlays: vec![testdata::popup_overlay()],
            ..toast("copied branch of #142")
        };
        insta::assert_snapshot!(draw(&model, 100, 30, &testdata::punk()));
    }
}
