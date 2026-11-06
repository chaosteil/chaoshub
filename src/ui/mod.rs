//! ratatui renderers for the models in `app::models`.
#![allow(dead_code, reason = "run calls the renderers in later phases")]

mod confirm;
mod filter;
mod help;
mod hints;
pub(crate) mod hits;
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
use crate::ui::hits::HitMap;

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

/// The parts of the list area: the header row, the rows and the filter
/// line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ListAreas {
    /// The header row: the first row, when the column headers are on.
    pub header: Option<Rect>,
    /// The rows of the list.
    pub body: Rect,
    /// The filter line: the last row, when the filter shows.
    pub filter: Option<Rect>,
}

/// Splits the list area into the header row, the rows and the filter
/// line. `view` draws the list there, and the scroll effect moves the
/// rows of `body`.
pub(crate) fn list_areas(model: &FrameModel, areas: &Areas) -> ListAreas {
    let mut body = rect(areas.list);
    let mut header = None;
    if model.column_headers && body.height > 0 {
        header = Some(Rect::new(body.x, body.y, body.width, 1));
        body.y += 1;
        body.height -= 1;
    }
    let mut filter = None;
    if model.filter.is_some() && body.height > 0 {
        body.height -= 1;
        filter = Some(Rect::new(body.x, body.bottom(), body.width, 1));
    }
    ListAreas {
        header,
        body,
        filter,
    }
}

/// The area of an overlay. `ui` draws the overlay there, and the
/// transitions slide it there.
pub(crate) fn overlay_area(overlay: &OverlayModel, areas: &Areas) -> Rect {
    let list = rect(areas.list);
    match &overlay.content {
        OverlayContent::Popup { .. } | OverlayContent::Output { .. } => rect(areas.popup),
        OverlayContent::Help { .. } => rect(areas.help),
        OverlayContent::Confirm(spec) => {
            let lines = u16::try_from(spec.lines.len()).unwrap_or(u16::MAX);
            // The frame, the lines, one blank row and the key hint.
            centered(list, DIALOG_WIDTH, lines.saturating_add(4))
        }
        OverlayContent::Prompt { .. } => centered(list, DIALOG_WIDTH, PROMPT_HEIGHT),
        OverlayContent::Picker { items, .. } => {
            let items = u16::try_from(items.len()).unwrap_or(u16::MAX);
            centered(list, PICKER_WIDTH, items.saturating_add(2))
        }
    }
}

/// Draws one frame: the tab bar, the list, the overlay stack, the key
/// hint bar and the status bar. The two bars come last, so the drop
/// shadow of an overlay does not cover them. `hits` gets the targets of
/// the frame, in draw order, so an overlay covers the targets below it.
pub(crate) fn view(
    frame: &mut Frame<'_>,
    model: &FrameModel,
    areas: &Areas,
    theme: &Theme,
    hits: &mut HitMap,
) {
    hits.clear();
    let buf = frame.buffer_mut();
    tabbar::render(&model.tab_bar, rect(areas.tab_bar), buf, theme, hits);
    let parts = list_areas(model, areas);
    if let Some(header) = parts.header {
        list::render_header(&model.list.columns, header, buf, theme);
    }
    if let (Some(line), Some(line_area)) = (&model.filter, parts.filter) {
        filter::render(line, line_area, buf, theme);
    }
    list::render(&model.list, parts.body, buf, theme, hits);
    for overlay in &model.overlays {
        render_overlay(overlay, areas, buf, theme, hits);
    }
    if let Some(model) = &model.hints {
        hints::render(model, rect(areas.hints), buf, theme, hits);
    }
    status::render(&model.status, rect(areas.status), buf, theme);
}

/// Draws one overlay into its area.
pub(crate) fn render_overlay(
    overlay: &OverlayModel,
    areas: &Areas,
    buf: &mut ratatui::buffer::Buffer,
    theme: &Theme,
    hits: &mut HitMap,
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
            popup::render(&content, area, buf, theme, hits);
        }
        OverlayContent::Help { help, scroll } => {
            help::render(help, *scroll, area, buf, theme, hits);
        }
        OverlayContent::Prompt { spec, text } => {
            prompt::render(spec, text, area, buf, theme, hits);
        }
        OverlayContent::Output {
            title,
            lines,
            scroll,
        } => {
            let content = output::Output {
                title,
                lines,
                scroll: *scroll,
            };
            output::render(&content, area, buf, theme, hits);
        }
        OverlayContent::Confirm(spec) => confirm::render(spec, area, buf, theme, hits),
        OverlayContent::Picker {
            title,
            items,
            cursor,
        } => picker::render(title, items, *cursor, area, buf, theme, hits),
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
            .draw(|frame| view(frame, model, &areas, theme, &mut HitMap::default()))
            .unwrap();
        terminal.backend().to_string()
    }

    /// The hit map of a frame model drawn at 100 by 30.
    fn hits(model: &FrameModel) -> HitMap {
        let mut terminal = testdata::terminal(100, 30);
        let areas = testdata::areas(100, 30);
        let mut hits = HitMap::default();
        terminal
            .draw(|frame| view(frame, model, &areas, &testdata::pierre(), &mut hits))
            .unwrap();
        hits
    }

    #[test]
    fn the_header_row_moves_the_list_down_one_row() {
        let areas = testdata::areas(100, 30);
        let list = rect(areas.list);
        let off = testdata::frame();
        assert_eq!(
            list_areas(&off, &areas),
            ListAreas {
                header: None,
                body: list,
                filter: None,
            }
        );
        let on = FrameModel {
            column_headers: true,
            filter: Some(crate::app::models::FilterModel {
                text: "x".into(),
                editing: false,
            }),
            ..testdata::frame()
        };
        let parts = list_areas(&on, &areas);
        assert_eq!(parts.header, Some(Rect::new(0, list.y, 100, 1)));
        assert_eq!(
            parts.body,
            Rect::new(0, list.y + 1, 100, list.height - 2),
            "one row lower and one row shorter, and the filter line"
        );
        assert_eq!(parts.filter, Some(Rect::new(0, list.bottom() - 1, 100, 1)));
        let on = FrameModel {
            column_headers: true,
            ..testdata::frame()
        };
        let screen = draw(&on, 100, 30, &testdata::pierre());
        let rows: Vec<&str> = screen.lines().collect();
        assert!(rows[list.y as usize].contains("TITLE"), "{screen}");
        let plain = draw(&off, 100, 30, &testdata::pierre());
        let plain: Vec<&str> = plain.lines().collect();
        assert_eq!(rows[list.y as usize + 1], plain[list.y as usize]);
        let hits = hits(&on);
        assert_eq!(hits.at(5, list.y), None, "the header row is no row");
        assert_eq!(hits.at(5, list.y + 1), Some(crate::bus::Hit::Row(0)));
    }

    #[test]
    fn the_sidebar_covers_the_list_rows_under_it() {
        let model = FrameModel {
            hints: Some(testdata::hints()),
            ..popup()
        };
        let hits = hits(&model);
        let areas = testdata::areas(100, 30);
        let popup = rect(areas.popup);
        let y = popup.y + 2;
        assert_eq!(hits.at(popup.x - 1, y), Some(crate::bus::Hit::Row(2)));
        assert_eq!(hits.at(popup.x, y), Some(crate::bus::Hit::Inside));
        assert_eq!(hits.at(2, 0), Some(crate::bus::Hit::Tab(0)));
        assert!(
            matches!(hits.at(2, areas.hints.y), Some(crate::bus::Hit::Key(_))),
            "the hint bar stays"
        );
        assert_eq!(hits.at(2, areas.status.y), None, "the status bar");
    }

    #[test]
    fn the_last_overlay_covers_the_ones_below() {
        let mut model = popup();
        model.overlays.push(testdata::picker_overlay(5, 1));
        let picker = overlay_area(&model.overlays[1], &testdata::areas(100, 30));
        let hits = hits(&model);
        assert_eq!(
            hits.at(picker.x + 2, picker.y + 1),
            Some(crate::bus::Hit::Item(0))
        );
        assert_eq!(
            hits.at(picker.x, picker.y + 1),
            Some(crate::bus::Hit::Inside)
        );
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
                        view(frame, model, &areas, &theme, &mut HitMap::default());
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
            .draw(|frame| view(frame, &model, &areas, &theme, &mut HitMap::default()))
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

    /// Checks that the key hint bar stays visible over an overlay in the
    /// punk theme at 100 by 30.
    fn assert_punk_hints_over(model: FrameModel) {
        let model = FrameModel {
            hints: Some(testdata::hints()),
            ..model
        };
        let theme = testdata::punk();
        let areas = testdata::areas(100, 30);
        let mut terminal = testdata::terminal(100, 30);
        let mut hits = HitMap::default();
        terminal
            .draw(|frame| view(frame, &model, &areas, &theme, &mut hits))
            .unwrap();
        let buf = terminal.backend().buffer();
        let y = areas.hints.y;
        let row: String = (0..100).map(|x| buf[(x, y)].symbol()).collect();
        assert!(row.contains("j/k move"), "{row}");
        for x in 0..100 {
            assert_eq!(buf[(x, y)].bg, theme.palette.bg, "x = {x}");
            assert_ne!(buf[(x, y)].bg, theme.palette.primary, "x = {x}");
        }
        let x = row[..row.find("j/k").unwrap()].chars().count();
        let x = u16::try_from(x).unwrap();
        assert!(
            matches!(hits.at(x, y), Some(crate::bus::Hit::Key(_))),
            "the hint is a key"
        );
    }

    #[test]
    fn punk_popup_shadow_keeps_the_hints() {
        assert_punk_hints_over(popup());
    }

    #[test]
    fn punk_help_shadow_keeps_the_hints() {
        assert_punk_hints_over(help());
    }

    #[test]
    fn punk_hints() {
        for (kind, theme) in punk_themes() {
            for (width, _) in PUNK_SIZES {
                let area = Rect::new(0, 0, width, 1);
                let mut buf = ratatui::buffer::Buffer::empty(area);
                hints::render(
                    &testdata::hints(),
                    area,
                    &mut buf,
                    &theme,
                    &mut HitMap::default(),
                );
                let row: String = (0..width).map(|x| buf[(x, 0)].symbol()).collect();
                insta::assert_snapshot!(format!("punk_hints_{width}_{kind}"), row);
            }
        }
    }
}
