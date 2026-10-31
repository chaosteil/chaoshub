//! ratatui renderers for the models in `app::models`.
#![allow(dead_code, reason = "run calls the renderers in later phases")]

mod confirm;
mod filter;
mod help;
mod list;
mod markdown;
mod output;
mod pane;
mod picker;
mod popup;
mod prompt;
mod status;
mod style;
mod tabbar;
#[cfg(test)]
mod testdata;
mod text;
pub(crate) mod transition;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::layout::{self, Areas};
use crate::app::models::{FrameModel, OverlayContent, OverlayModel};
use crate::theme::Theme;

/// The width of a confirm or prompt popup.
const DIALOG_WIDTH: u16 = 64;
/// The height of a prompt popup.
const PROMPT_HEIGHT: u16 = 8;
/// The width of a picker popup.
const PICKER_WIDTH: u16 = 40;

/// Converts a plain rect of `app::layout` to a ratatui rect.
pub(crate) fn rect(r: layout::Rect) -> Rect {
    Rect::new(r.x, r.y, r.width, r.height)
}

/// The whole screen of the areas.
pub(crate) fn screen(areas: &Areas) -> Rect {
    Rect::new(
        0,
        0,
        areas.list.width,
        areas.status.y.saturating_add(areas.status.height),
    )
}

/// A rect of `width` by `height`, centered in `outer` and clipped to it.
fn centered(outer: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(outer.width);
    let height = height.min(outer.height);
    Rect::new(
        outer.x + (outer.width - width) / 2,
        outer.y + (outer.height - height) / 2,
        width,
        height,
    )
}

/// The area of an overlay. `ui` draws the overlay there, and the
/// transitions slide it there.
pub(crate) fn overlay_area(overlay: &OverlayModel, areas: &Areas) -> Rect {
    let popup = rect(areas.popup);
    match &overlay.content {
        OverlayContent::Popup { .. } | OverlayContent::Output { .. } => popup,
        OverlayContent::Help(_) => rect(areas.help),
        OverlayContent::Confirm(spec) => {
            let lines = u16::try_from(spec.lines.len()).unwrap_or(u16::MAX);
            // The frame, the lines, one blank row and the key hint.
            centered(popup, DIALOG_WIDTH, lines.saturating_add(4))
        }
        OverlayContent::Prompt { .. } => centered(popup, DIALOG_WIDTH, PROMPT_HEIGHT),
        OverlayContent::Picker { items, .. } => {
            let items = u16::try_from(items.len()).unwrap_or(u16::MAX);
            centered(popup, PICKER_WIDTH, items.saturating_add(2))
        }
    }
}

/// Draws one frame: the tab bar, the list, the status bar and the overlay
/// stack.
pub(crate) fn view(frame: &mut Frame<'_>, model: &FrameModel, areas: &Areas, theme: &Theme) {
    let buf = frame.buffer_mut();
    tabbar::render(&model.tab_bar, rect(areas.tab_bar), buf, theme);
    list::render(&model.list, rect(areas.list), buf, theme);
    status::render(&model.status, rect(areas.status), buf, theme);
    for overlay in &model.overlays {
        render_overlay(overlay, areas, buf, theme);
    }
}

/// Draws one overlay into its area.
fn render_overlay(
    overlay: &OverlayModel,
    areas: &Areas,
    buf: &mut ratatui::buffer::Buffer,
    theme: &Theme,
) {
    let area = overlay_area(overlay, areas);
    match &overlay.content {
        OverlayContent::Popup {
            title,
            subtitle,
            tabs,
            active,
            pane,
            cursor,
            scroll,
        } => {
            let content = popup::Popup {
                title,
                subtitle,
                tabs,
                active: *active,
                pane,
                cursor: *cursor,
                scroll: *scroll,
            };
            popup::render(&content, area, buf, theme);
        }
        OverlayContent::Help(help) => help::render(help, area, buf, theme),
        // The renderers of the other overlay kinds come in the next
        // commits of this stack.
        OverlayContent::Confirm(_)
        | OverlayContent::Prompt { .. }
        | OverlayContent::Output { .. }
        | OverlayContent::Picker { .. } => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testdata;

    /// Draws a frame model at a size with a theme.
    pub(crate) fn draw(model: &FrameModel, width: u16, height: u16, theme: &Theme) -> String {
        let mut terminal = testdata::terminal(width, height);
        let areas = testdata::areas(width, height);
        terminal
            .draw(|frame| view(frame, model, &areas, theme))
            .unwrap();
        terminal.backend().to_string()
    }

    #[test]
    fn main_screen_100_pierre() {
        insta::assert_snapshot!(draw(&testdata::frame(), 100, 30, &testdata::pierre()));
    }

    #[test]
    fn main_screen_160_pierre() {
        insta::assert_snapshot!(draw(&testdata::frame(), 160, 40, &testdata::pierre()));
    }

    #[test]
    fn main_screen_100_ansi16() {
        insta::assert_snapshot!(draw(&testdata::frame(), 100, 30, &testdata::ansi16()));
    }

    #[test]
    fn main_screen_160_ansi16() {
        insta::assert_snapshot!(draw(&testdata::frame(), 160, 40, &testdata::ansi16()));
    }

    fn popup() -> FrameModel {
        testdata::with_overlay(testdata::popup_overlay())
    }

    #[test]
    fn popup_100_pierre() {
        insta::assert_snapshot!(draw(&popup(), 100, 30, &testdata::pierre()));
    }

    #[test]
    fn popup_160_pierre() {
        insta::assert_snapshot!(draw(&popup(), 160, 40, &testdata::pierre()));
    }

    #[test]
    fn popup_100_ansi16() {
        insta::assert_snapshot!(draw(&popup(), 100, 30, &testdata::ansi16()));
    }

    #[test]
    fn popup_160_ansi16() {
        insta::assert_snapshot!(draw(&popup(), 160, 40, &testdata::ansi16()));
    }

    /// Draws frames through `view` and the transitions, one per elapsed
    /// step, with a fixed clock.
    pub(crate) fn animate(
        model: &FrameModel,
        events: Vec<crate::bus::UiEvent>,
        steps_ms: &[u64],
    ) -> Vec<String> {
        let (width, height) = (100, 30);
        let theme = testdata::pierre();
        let areas = testdata::areas(width, height);
        let mut terminal = testdata::terminal(width, height);
        let mut fx = transition::Transitions::new(true, 160).with_theme(theme);
        fx.apply(events);
        steps_ms
            .iter()
            .map(|&ms| {
                fx.prepare(model, &areas);
                terminal
                    .draw(|frame| {
                        view(frame, model, &areas, &theme);
                        fx.process(frame, std::time::Duration::from_millis(ms));
                    })
                    .unwrap();
                terminal.backend().to_string()
            })
            .collect()
    }

    #[test]
    fn popup_slides_in() {
        let frames = animate(
            &popup(),
            vec![crate::bus::UiEvent::OverlayOpened(
                crate::bus::OverlayKind::Popup,
            )],
            &[0, 80, 80],
        );
        insta::assert_snapshot!("popup_slide_in_progress_0.0", frames[0]);
        insta::assert_snapshot!("popup_slide_in_progress_0.5", frames[1]);
        insta::assert_snapshot!("popup_slide_in_progress_1.0", frames[2]);
    }

    fn help() -> FrameModel {
        testdata::with_overlay(testdata::help_overlay())
    }

    #[test]
    fn help_100_pierre() {
        insta::assert_snapshot!(draw(&help(), 100, 30, &testdata::pierre()));
    }

    #[test]
    fn help_160_pierre() {
        insta::assert_snapshot!(draw(&help(), 160, 40, &testdata::pierre()));
    }

    #[test]
    fn help_100_ansi16() {
        insta::assert_snapshot!(draw(&help(), 100, 30, &testdata::ansi16()));
    }

    #[test]
    fn help_160_ansi16() {
        insta::assert_snapshot!(draw(&help(), 160, 40, &testdata::ansi16()));
    }

    #[test]
    fn help_slides_in_from_the_right() {
        let frames = animate(
            &help(),
            vec![crate::bus::UiEvent::OverlayOpened(
                crate::bus::OverlayKind::Help,
            )],
            &[0, 80, 80],
        );
        insta::assert_snapshot!("help_slide_in_0ms", frames[0]);
        insta::assert_snapshot!("help_slide_in_80ms", frames[1]);
        insta::assert_snapshot!("help_slide_in_160ms", frames[2]);
    }

    #[test]
    fn popup_slides_out() {
        let mut model = popup();
        model.overlays[0].closing = true;
        let frames = animate(
            &model,
            vec![crate::bus::UiEvent::OverlayClosed(
                crate::bus::OverlayKind::Popup,
            )],
            &[0, 80, 80],
        );
        insta::assert_snapshot!("popup_slide_out_progress_0.0", frames[0]);
        insta::assert_snapshot!("popup_slide_out_progress_0.5", frames[1]);
        insta::assert_snapshot!("popup_slide_out_progress_1.0", frames[2]);
    }
}
