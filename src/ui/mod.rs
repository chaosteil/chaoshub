//! ratatui renderers for the models in `app::models`.
#![allow(dead_code, reason = "run calls the renderers in later phases")]

mod confirm;
mod filter;
mod help;
mod hints;
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

/// Draws one frame: the tab bar, the list, the key hint bar, the status
/// bar and the overlay stack.
pub(crate) fn view(frame: &mut Frame<'_>, model: &FrameModel, areas: &Areas, theme: &Theme) {
    let buf = frame.buffer_mut();
    tabbar::render(&model.tab_bar, rect(areas.tab_bar), buf, theme);
    let mut list_area = rect(areas.list);
    if let Some(line) = &model.filter
        && list_area.height > 0
    {
        list_area.height -= 1;
        let line_area = Rect::new(list_area.x, list_area.bottom(), list_area.width, 1);
        filter::render(line, line_area, buf, theme);
    }
    list::render(&model.list, list_area, buf, theme);
    if let Some(model) = &model.hints {
        hints::render(model, rect(areas.hints), buf, theme);
    }
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
        OverlayContent::Prompt { spec, text } => prompt::render(spec, text, area, buf, theme),
        OverlayContent::Output { title, lines } => output::render(title, lines, area, buf, theme),
        OverlayContent::Confirm(spec) => confirm::render(spec, area, buf, theme),
        OverlayContent::Picker {
            title,
            items,
            cursor,
        } => picker::render(title, items, *cursor, area, buf, theme),
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
    fn filter_line_takes_the_last_list_row() {
        let mut model = testdata::frame();
        model.filter = Some(crate::app::models::FilterModel {
            text: "pane".into(),
            editing: true,
        });
        insta::assert_snapshot!(draw(&model, 100, 12, &testdata::ansi16()));
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

    fn prompt(text: &str) -> FrameModel {
        testdata::with_overlay(testdata::prompt_overlay(text))
    }

    #[test]
    fn prompt_100_pierre() {
        let text = "Please split the transition module into two commits, one for the \
                    mapping and one for the scratch buffers.";
        insta::assert_snapshot!(draw(&prompt(text), 100, 30, &testdata::pierre()));
    }

    #[test]
    fn prompt_160_pierre() {
        insta::assert_snapshot!(draw(
            &prompt("line one\nline two"),
            160,
            40,
            &testdata::pierre()
        ));
    }

    #[test]
    fn prompt_100_ansi16() {
        insta::assert_snapshot!(draw(&prompt(""), 100, 30, &testdata::ansi16()));
    }

    #[test]
    fn prompt_160_ansi16() {
        insta::assert_snapshot!(draw(&prompt("ok"), 160, 40, &testdata::ansi16()));
    }

    fn output(count: usize) -> FrameModel {
        testdata::with_overlay(testdata::output_overlay(count))
    }

    #[test]
    fn output_100_pierre() {
        insta::assert_snapshot!(draw(&output(5), 100, 30, &testdata::pierre()));
    }

    #[test]
    fn output_160_pierre() {
        insta::assert_snapshot!(draw(&output(5), 160, 40, &testdata::pierre()));
    }

    #[test]
    fn output_100_ansi16() {
        insta::assert_snapshot!(draw(&output(5), 100, 30, &testdata::ansi16()));
    }

    #[test]
    fn output_160_ansi16() {
        insta::assert_snapshot!(draw(&output(5), 160, 40, &testdata::ansi16()));
    }

    #[test]
    fn output_follows_the_tail() {
        insta::assert_snapshot!(draw(&output(60), 100, 30, &testdata::ansi16()));
    }

    #[test]
    fn confirm_100_pierre() {
        let model = testdata::with_overlay(testdata::confirm_overlay(true));
        insta::assert_snapshot!(draw(&model, 100, 30, &testdata::pierre()));
    }

    #[test]
    fn confirm_160_pierre() {
        let model = testdata::with_overlay(testdata::confirm_overlay(false));
        insta::assert_snapshot!(draw(&model, 160, 40, &testdata::pierre()));
    }

    #[test]
    fn confirm_100_ansi16() {
        let model = testdata::with_overlay(testdata::confirm_overlay(true));
        insta::assert_snapshot!(draw(&model, 100, 30, &testdata::ansi16()));
    }

    #[test]
    fn confirm_160_ansi16() {
        let model = testdata::with_overlay(testdata::confirm_overlay(true));
        insta::assert_snapshot!(draw(&model, 160, 40, &testdata::ansi16()));
    }

    #[test]
    fn danger_confirm_uses_the_danger_color() {
        let theme = testdata::pierre();
        let model = testdata::with_overlay(testdata::confirm_overlay(true));
        let areas = testdata::areas(100, 30);
        let mut terminal = testdata::terminal(100, 30);
        terminal
            .draw(|frame| view(frame, &model, &areas, &theme))
            .unwrap();
        let area = overlay_area(&model.overlays[0], &areas);
        let corner = &terminal.backend().buffer()[(area.x, area.y)];
        assert_eq!(corner.fg, theme.palette.danger);
    }

    #[test]
    fn picker_100_pierre() {
        let model = testdata::with_overlay(testdata::picker_overlay(5, 1));
        insta::assert_snapshot!(draw(&model, 100, 30, &testdata::pierre()));
    }

    #[test]
    fn picker_160_pierre() {
        let model = testdata::with_overlay(testdata::picker_overlay(5, 1));
        insta::assert_snapshot!(draw(&model, 160, 40, &testdata::pierre()));
    }

    #[test]
    fn picker_100_ansi16() {
        let model = testdata::with_overlay(testdata::picker_overlay(5, 1));
        insta::assert_snapshot!(draw(&model, 100, 30, &testdata::ansi16()));
    }

    #[test]
    fn picker_160_ansi16() {
        let model = testdata::with_overlay(testdata::picker_overlay(5, 1));
        insta::assert_snapshot!(draw(&model, 160, 40, &testdata::ansi16()));
    }

    #[test]
    fn picker_scrolls_to_the_cursor() {
        let model = testdata::with_overlay(testdata::picker_overlay(40, 35));
        insta::assert_snapshot!(draw(&model, 100, 30, &testdata::ansi16()));
    }

    #[test]
    fn overlays_stack_bottom_first() {
        let mut model = testdata::with_overlay(testdata::popup_overlay());
        model.overlays.push(testdata::confirm_overlay(true));
        insta::assert_snapshot!(draw(&model, 100, 30, &testdata::ansi16()));
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

    /// The punk themes with their snapshot names.
    fn punk_themes() -> [(&'static str, Theme); 2] {
        [
            ("truecolor", testdata::punk()),
            ("ansi16", testdata::punk_ansi16()),
        ]
    }

    /// The screen sizes of the punk snapshots.
    const PUNK_SIZES: [(u16, u16); 2] = [(100, 30), (160, 40)];

    /// Snapshots a frame model in both punk themes at both sizes.
    fn punk_snapshots(name: &str, model: &FrameModel) {
        for (kind, theme) in punk_themes() {
            for (width, height) in PUNK_SIZES {
                insta::assert_snapshot!(
                    format!("punk_{name}_{width}_{kind}"),
                    draw(model, width, height, &theme)
                );
            }
        }
    }

    #[test]
    fn punk_list() {
        let model = FrameModel {
            hints: Some(testdata::hints()),
            ..testdata::frame()
        };
        punk_snapshots("list", &model);
    }

    #[test]
    fn punk_popup() {
        punk_snapshots("popup", &popup());
    }

    #[test]
    fn punk_confirm() {
        punk_snapshots(
            "confirm",
            &testdata::with_overlay(testdata::confirm_overlay(true)),
        );
    }

    #[test]
    fn punk_help() {
        punk_snapshots("help", &help());
    }

    #[test]
    fn punk_hints() {
        for (kind, theme) in punk_themes() {
            for (width, _) in PUNK_SIZES {
                let area = Rect::new(0, 0, width, 1);
                let mut buf = ratatui::buffer::Buffer::empty(area);
                hints::render(&testdata::hints(), area, &mut buf, &theme);
                let row: String = (0..width).map(|x| buf[(x, 0)].symbol()).collect();
                insta::assert_snapshot!(format!("punk_hints_{width}_{kind}"), row);
            }
        }
    }
}
