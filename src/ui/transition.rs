//! Transitions on tachyonfx. `update` pushes plain `UiEvent`s, and
//! `Transitions` turns them into effects that run over the frame buffer
//! after `view`.
//!
//! - Tab switch: the highlight of the active tab glides from the old tab
//!   to the new tab in the tab bar row. The list shows the new tab at
//!   once.
//! - Confirm, prompt and picker: `slide_in` from the bottom edge on open,
//!   `slide_out` on close.
//! - Popup, output and help: `slide_in` from the right edge, `slide_out`
//!   to the right.
//! - Scroll: the list rows move from the old offset to the new one.
//! - Expired status message: `fade_to_fg` toward the status bar grey.

use std::rc::Rc;
use std::time::Duration;

use ratatui::Frame;
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::{Offset, Rect};
use tachyonfx::{
    EffectManager, EffectTimer, Interpolation, Motion, RefCount, RefRect, fx, ref_count,
};

use crate::app::layout::Areas;
use crate::app::models::{FrameModel, TabBarModel};
use crate::bus::{Hit, OverlayKind, UiEvent};
use crate::theme::{Theme, ThemeKind};
use crate::ui::hits::HitMap;
use crate::ui::{list, overlay_area, rect, tabbar};

/// The duration of the confirm and prompt slides, in milliseconds.
const DIALOG_MS: u64 = 100;
/// The duration of the status fade, in milliseconds.
const STATUS_FADE_MS: u64 = 300;
/// The duration of the tab glide, in milliseconds.
const TAB_GLIDE_MS: u64 = 120;
/// The length of the gradient at the edge of a sliding overlay.
const SLIDE_GRADIENT: u16 = 3;
/// The frame deadline of the event loop while a transition runs.
const FRAME_DEADLINE: Duration = Duration::from_millis(16);
/// The frame deadline of the event loop while no transition runs.
const IDLE_DEADLINE: Duration = Duration::from_secs(1);

/// The key of an effect. A new effect with the key of a running effect
/// replaces it, so a key press during a transition retargets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
enum FxKey {
    #[default]
    Tab,
    Scroll,
    Status,
    Overlay(usize),
}

/// The number of overlay kinds.
const OVERLAY_KINDS: usize = 6;

/// The slot of an overlay kind, for its key and its area.
fn slot(kind: OverlayKind) -> usize {
    match kind {
        OverlayKind::Popup => 0,
        OverlayKind::Confirm => 1,
        OverlayKind::Prompt => 2,
        OverlayKind::Output => 3,
        OverlayKind::Picker => 4,
        OverlayKind::Help => 5,
    }
}

/// The key of the effect for an event.
fn key(event: &UiEvent) -> FxKey {
    match event {
        UiEvent::TabChanged { .. } => FxKey::Tab,
        UiEvent::OverlayOpened(kind) | UiEvent::OverlayClosed(kind) => FxKey::Overlay(slot(*kind)),
        UiEvent::ScrollTo { .. } => FxKey::Scroll,
        UiEvent::StatusExpired => FxKey::Status,
    }
}

/// The state of a scroll effect.
#[derive(Debug, Clone)]
struct ScrollState {
    /// The list rows from the upper offset to the lower offset plus one
    /// screen of rows.
    rows: RefCount<Buffer>,
    /// The list area.
    list: RefRect,
    /// True when the list scrolls down.
    down: bool,
}

/// The state of a tab glide.
#[derive(Debug, Clone, Default)]
struct TabGlide {
    /// The tab bar row with no active tab.
    bar: Buffer,
    /// The x and the width of the old active tab.
    from: (u16, u16),
    /// The x and the width of the new active tab.
    to: (u16, u16),
}

impl TabGlide {
    /// The cells `[l, r)` of the highlight at `alpha`: from the span of
    /// the old tab at 0 to the span of the new tab at 1.
    fn span(&self, alpha: f32) -> (u16, u16) {
        let alpha = alpha.clamp(0.0, 1.0);
        let lerp = |a: u16, b: u16| {
            let x = (f32::from(a) + (f32::from(b) - f32::from(a)) * alpha).round();
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "x is rounded and between a and b"
            )]
            let x = x as u16;
            x
        };
        let (from_x, from_width) = self.from;
        let (to_x, to_width) = self.to;
        (
            lerp(from_x, to_x),
            lerp(from_x + from_width, to_x + to_width),
        )
    }
}

/// Turns UI events into effects and runs them over the frame buffer.
#[derive(Debug)]
pub(crate) struct Transitions {
    /// False when the configuration turns animations off.
    enabled: bool,
    /// The duration of one transition.
    duration: Duration,
    theme: Theme,
    manager: EffectManager<FxKey>,
    /// The areas of the last `prepare`.
    areas: Option<Areas>,
    /// The list area, shared with the scroll effect.
    list: RefRect,
    /// The area of each overlay kind, shared with its slide effect.
    overlays: [RefRect; OVERLAY_KINDS],
    /// The state of the running tab glide.
    tab_glide: RefCount<TabGlide>,
    /// The old and the new active tab of a tab glide that `prepare` must
    /// fill in.
    pending_tab: Option<(usize, usize)>,
    /// The list rows during a scroll.
    rows: RefCount<Buffer>,
    /// The offsets of the running scroll.
    scroll: Option<(usize, usize)>,
    /// True when a status message expired and its fade must start.
    status_pending: bool,
    /// The status row of the last frame.
    status_row: Option<Buffer>,
}

impl Transitions {
    /// Creates the transitions. `enabled` comes from `[theme] animations`
    /// and `duration_ms` from `[theme] animation_ms`.
    pub(crate) fn new(enabled: bool, duration_ms: u64) -> Self {
        let empty = || ref_count(Buffer::empty(Rect::default()));
        Self {
            enabled,
            duration: Duration::from_millis(duration_ms),
            theme: Theme::new(ThemeKind::TrueColor, true, true),
            manager: EffectManager::default(),
            areas: None,
            list: RefRect::new(Rect::default()),
            overlays: std::array::from_fn(|_| RefRect::new(Rect::default())),
            tab_glide: ref_count(TabGlide::default()),
            pending_tab: None,
            rows: empty(),
            scroll: None,
            status_pending: false,
            status_row: None,
        }
    }

    /// Sets the theme for the colors behind sliding cells, the status fade
    /// and the previous list.
    #[must_use]
    pub(crate) fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// A timer of `duration` with the default easing.
    fn timer(duration: Duration) -> EffectTimer {
        EffectTimer::new(duration.into(), Interpolation::CubicOut)
    }

    /// Starts one effect per event. A running effect with the same key is
    /// replaced. With animations off, it drops every event.
    pub(crate) fn apply(&mut self, events: Vec<UiEvent>) {
        if !self.enabled {
            return;
        }
        for event in events {
            let key = key(&event);
            match event {
                UiEvent::TabChanged { from, to } => self.tab_slide(key, from, to),
                UiEvent::OverlayOpened(kind) => self.overlay_slide(key, kind, true),
                UiEvent::OverlayClosed(kind) => self.overlay_slide(key, kind, false),
                UiEvent::ScrollTo { from, to, .. } => self.scroll(key, from, to),
                UiEvent::StatusExpired => self.status_pending = true,
            }
        }
    }

    /// The highlight of the active tab glides from the old tab to the new
    /// tab over the tab bar row. `prepare` draws the bar with no active tab
    /// and finds the spans of the two tabs. Each frame, the effect copies
    /// that bar over the tab bar and gives the cells of the highlight the
    /// active tab colors. The list does not move.
    fn tab_slide(&mut self, key: FxKey, from: usize, to: usize) {
        self.pending_tab = Some((from, to));
        // A new state for each glide: a replaced glide keeps its own state
        // for its last frame.
        self.tab_glide = ref_count(TabGlide::default());
        let (bg, fg) = self.theme.active_tab_colors();
        let timer = Self::timer(Duration::from_millis(TAB_GLIDE_MS));
        let effect = fx::effect_fn_buf(Rc::clone(&self.tab_glide), timer, move |glide, cx, buf| {
            let glide = glide.borrow();
            let (l, r) = glide.span(cx.alpha());
            for (x, y) in positions(glide.bar.area) {
                let Some(dst) = buf.cell_mut((x, y)) else {
                    continue;
                };
                *dst = glide.bar[(x, y)].clone();
                if (l..r).contains(&x) {
                    dst.set_bg(bg).set_fg(fg);
                }
            }
        });
        self.manager.add_unique_effect(key, effect);
    }

    /// An overlay slides in or out of its area.
    fn overlay_slide(&mut self, key: FxKey, kind: OverlayKind, open: bool) {
        let duration = match kind {
            OverlayKind::Confirm | OverlayKind::Prompt => Duration::from_millis(DIALOG_MS),
            _ => self.duration,
        };
        let timer = Self::timer(duration);
        // `slide_in` runs `slide_out` backwards, so an ease-in curve there
        // looks like the ease-out curve of the other transitions.
        let in_timer = EffectTimer::new(duration.into(), Interpolation::CubicIn);
        let behind = self.theme.palette.bg;
        let effect = match (kind, open) {
            (OverlayKind::Popup | OverlayKind::Output | OverlayKind::Help, true) => {
                fx::slide_in(Motion::RightToLeft, SLIDE_GRADIENT, 0, behind, in_timer)
            }
            (OverlayKind::Popup | OverlayKind::Output | OverlayKind::Help, false) => {
                fx::slide_out(Motion::LeftToRight, SLIDE_GRADIENT, 0, behind, timer)
            }
            (_, true) => fx::slide_in(Motion::DownToUp, SLIDE_GRADIENT, 0, behind, in_timer),
            (_, false) => fx::slide_out(Motion::UpToDown, SLIDE_GRADIENT, 0, behind, timer),
        };
        let area = self.overlays[slot(kind)].clone();
        self.manager
            .add_unique_effect(key, fx::dynamic_area(area, effect));
    }

    /// The list rows move from the old offset to the new one.
    fn scroll(&mut self, key: FxKey, from: usize, to: usize) {
        if from == to {
            return;
        }
        self.scroll = Some((from, to));
        let state = ScrollState {
            rows: Rc::clone(&self.rows),
            list: self.list.clone(),
            down: to > from,
        };
        let effect = fx::effect_fn_buf(state, Self::timer(self.duration), |state, cx, buf| {
            let list = state.list.get();
            let rows = state.rows.borrow();
            let distance = rows.area.height.saturating_sub(list.height);
            let alpha = if state.down {
                cx.alpha()
            } else {
                1.0 - cx.alpha()
            };
            let top = (f32::from(distance) * alpha.clamp(0.0, 1.0)).round();
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "top is rounded and between 0 and distance"
            )]
            let top = top as u16;
            for dy in 0..list.height {
                for dx in 0..list.width {
                    let Some(src) = rows.cell((list.x + dx, top + dy)) else {
                        continue;
                    };
                    if let Some(dst) = buf.cell_mut((list.x + dx, list.y + dy)) {
                        *dst = src.clone();
                    }
                }
            }
        });
        self.manager.add_unique_effect(key, effect);
    }

    /// Draws the tab bar of a new tab glide and the scroll rows into the
    /// scratch buffers, and updates the areas of the running effects.
    pub(crate) fn prepare(&mut self, model: &FrameModel, areas: &Areas) {
        self.areas = Some(*areas);
        if !self.enabled {
            return;
        }
        let list_area = rect(areas.list);
        self.list.set(list_area);
        for overlay in &model.overlays {
            self.overlays[slot(overlay.kind)].set(overlay_area(overlay, areas));
        }
        if let Some((from, to)) = self.pending_tab.take() {
            let bar_area = rect(areas.tab_bar);
            let mut bar = Buffer::empty(bar_area);
            let mut hits = HitMap::default();
            let idle = TabBarModel {
                titles: model.tab_bar.titles.clone(),
                active: model.tab_bar.titles.len(),
            };
            tabbar::render(&idle, bar_area, &mut bar, &self.theme, &mut hits);
            let span = |i| hits.region(Hit::Tab(i)).map(|r| (r.x, r.width));
            if let (Some(from), Some(to)) = (span(from), span(to)) {
                *self.tab_glide.borrow_mut() = TabGlide { bar, from, to };
            }
        }
        if Rc::strong_count(&self.rows) > 1
            && let Some((from, to)) = self.scroll
        {
            *self.rows.borrow_mut() = scroll_rows(&model.list, list_area, from, to, &self.theme);
        }
    }

    /// Runs the effects over the frame buffer. `elapsed` is the time since
    /// the last call.
    pub(crate) fn process(&mut self, frame: &mut Frame<'_>, elapsed: Duration) {
        let area = frame.area();
        let buf = frame.buffer_mut();
        if let Some(areas) = self.areas {
            let status = rect(areas.status).intersection(buf.area);
            if std::mem::take(&mut self.status_pending) {
                self.status_fade(buf, status);
            }
            self.status_row = Some(copy(buf, status));
        }
        self.manager.process_effects(elapsed.into(), buf, area);
    }

    /// Starts the fade of an expired status message. The message is gone
    /// from the new frame, so the fade draws the cells of the last frame
    /// that differ from the new frame, and fades them out.
    fn status_fade(&mut self, buf: &Buffer, status: Rect) {
        let Some(old) = &self.status_row else {
            return;
        };
        if old.area != status {
            return;
        }
        let changed: Vec<u16> = (status.left()..status.right())
            .filter(|&x| (status.top()..status.bottom()).any(|y| old[(x, y)] != buf[(x, y)]))
            .collect();
        let (Some(&first), Some(&last)) = (changed.first(), changed.last()) else {
            return;
        };
        let span = Rect::new(first, status.y, last - first + 1, status.height);
        let mut masked = Buffer::empty(buf.area);
        for (x, y) in positions(span) {
            masked[(x, y)] = old[(x, y)].clone();
        }
        skip_outside(&mut masked, span);
        let timer = Self::timer(Duration::from_millis(STATUS_FADE_MS));
        let effect = fx::parallel(&[
            fx::translate_buf(Offset::default(), ref_count(masked), timer),
            fx::fade_to_fg(self.theme.palette.surface, timer).with_area(span),
        ]);
        self.manager.add_unique_effect(FxKey::Status, effect);
    }

    /// Turns the transitions off for the rest of the session and stops
    /// the running effects.
    pub(crate) fn disable(&mut self) {
        self.enabled = false;
        self.cancel();
    }

    /// Stops the running effects and the pending status fade. The next
    /// frame shows the end state.
    pub(crate) fn cancel(&mut self) {
        self.manager = EffectManager::default();
        self.scroll = None;
        self.status_pending = false;
    }

    /// True while a transition runs.
    pub(crate) fn active(&self) -> bool {
        self.manager.is_running() || self.status_pending
    }

    /// How long the event loop waits for a message: one frame while a
    /// transition runs, else one second.
    pub(crate) fn deadline(&self) -> Duration {
        if self.active() {
            FRAME_DEADLINE
        } else {
            IDLE_DEADLINE
        }
    }
}

/// The positions of the cells of an area, row by row.
fn positions(area: Rect) -> impl Iterator<Item = (u16, u16)> {
    (area.top()..area.bottom()).flat_map(move |y| (area.left()..area.right()).map(move |x| (x, y)))
}

/// A copy of the cells of an area.
fn copy(buf: &Buffer, area: Rect) -> Buffer {
    let mut out = Buffer::empty(area);
    for (x, y) in positions(area) {
        out[(x, y)] = buf[(x, y)].clone();
    }
    out
}

/// Marks every cell outside `keep` to skip, so a blit leaves the
/// destination cells there as they are.
fn skip_outside(buf: &mut Buffer, keep: Rect) {
    for (x, y) in positions(buf.area) {
        if !keep.contains((x, y).into()) {
            buf[(x, y)].diff_option = CellDiffOption::Skip;
        }
    }
}

/// The list rows that a scroll from `from` to `to` passes, plus one screen
/// of rows. A jump longer than one screen starts one screen before the
/// target.
fn scroll_rows(
    model: &crate::app::models::ListModel,
    list: Rect,
    from: usize,
    to: usize,
    theme: &Theme,
) -> Buffer {
    let height = usize::from(list.height);
    let distance = from.abs_diff(to).min(height);
    let top = if to > from { to - distance } else { to };
    let tall = u16::try_from(height + distance).unwrap_or(u16::MAX);
    let area = Rect::new(list.x, 0, list.width, tall);
    let mut buf = Buffer::empty(area);
    let mut window = model.clone();
    window.scroll = top;
    list::render(&window, area, &mut buf, theme, &mut HitMap::default());
    buf
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;
    use crate::app::models::{OverlayContent, OverlayModel};
    use crate::ui::testdata;

    const WIDTH: u16 = 100;
    const HEIGHT: u16 = 30;

    /// Draws one frame: the list, a stand-in overlay box and the status
    /// row, then the effects.
    fn draw(
        terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>,
        transitions: &mut Transitions,
        model: &FrameModel,
        elapsed_ms: u64,
    ) -> String {
        let areas = testdata::areas(WIDTH, HEIGHT);
        let theme = testdata::pierre();
        transitions.prepare(model, &areas);
        terminal
            .draw(|frame| {
                list::render(
                    &model.list,
                    rect(areas.list),
                    frame.buffer_mut(),
                    &theme,
                    &mut HitMap::default(),
                );
                for overlay in &model.overlays {
                    let area = overlay_area(overlay, &areas);
                    for (x, y) in positions(area) {
                        frame.buffer_mut()[(x, y)].set_char('#');
                    }
                }
                if let Some(center) = &model.status.center {
                    let status = rect(areas.status);
                    frame.buffer_mut().set_string(
                        status.x + 40,
                        status.y,
                        &center.text,
                        ratatui::style::Style::new(),
                    );
                }
                transitions.process(frame, Duration::from_millis(elapsed_ms));
            })
            .unwrap();
        terminal.backend().to_string()
    }

    fn overlay(kind: OverlayKind) -> OverlayModel {
        let content = match kind {
            OverlayKind::Help => OverlayContent::Help {
                help: crate::app::models::HelpModel {
                    sections: Vec::new(),
                },
                scroll: 0,
            },
            _ => OverlayContent::Output {
                title: "output".to_owned(),
                lines: Vec::new(),
                scroll: 0,
            },
        };
        OverlayModel {
            kind,
            closing: false,
            content,
        }
    }

    fn with_overlay(kind: OverlayKind) -> FrameModel {
        FrameModel {
            overlays: vec![overlay(kind)],
            ..testdata::frame()
        }
    }

    fn new() -> Transitions {
        Transitions::new(true, 160).with_theme(testdata::pierre())
    }

    #[test]
    fn events_map_to_keys() {
        assert_eq!(key(&UiEvent::TabChanged { from: 0, to: 1 }), FxKey::Tab);
        assert_eq!(
            key(&UiEvent::OverlayOpened(OverlayKind::Popup)),
            key(&UiEvent::OverlayClosed(OverlayKind::Popup))
        );
        assert_ne!(
            key(&UiEvent::OverlayOpened(OverlayKind::Popup)),
            key(&UiEvent::OverlayOpened(OverlayKind::Help))
        );
        assert_eq!(
            key(&UiEvent::ScrollTo {
                tab: "mine",
                from: 0,
                to: 3
            }),
            FxKey::Scroll
        );
        assert_eq!(key(&UiEvent::StatusExpired), FxKey::Status);
    }

    #[test]
    fn active_until_the_duration_ends() {
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let mut fx = new();
        let model = with_overlay(OverlayKind::Popup);
        assert!(!fx.active());
        assert_eq!(fx.deadline(), IDLE_DEADLINE);
        fx.apply(vec![UiEvent::OverlayOpened(OverlayKind::Popup)]);
        assert!(fx.active());
        assert_eq!(fx.deadline(), FRAME_DEADLINE);
        draw(&mut terminal, &mut fx, &model, 0);
        draw(&mut terminal, &mut fx, &model, 159);
        assert!(fx.active());
        draw(&mut terminal, &mut fx, &model, 1);
        assert!(!fx.active());
    }

    #[test]
    fn dialogs_take_100_ms() {
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let mut fx = new();
        let model = with_overlay(OverlayKind::Prompt);
        fx.apply(vec![UiEvent::OverlayOpened(OverlayKind::Prompt)]);
        draw(&mut terminal, &mut fx, &model, 0);
        draw(&mut terminal, &mut fx, &model, 99);
        assert!(fx.active());
        draw(&mut terminal, &mut fx, &model, 1);
        assert!(!fx.active());
    }

    #[test]
    fn disabled_drops_every_event() {
        let mut fx = Transitions::new(false, 160);
        fx.apply(vec![
            UiEvent::TabChanged { from: 0, to: 1 },
            UiEvent::OverlayOpened(OverlayKind::Popup),
            UiEvent::StatusExpired,
        ]);
        assert!(!fx.active());
    }

    #[test]
    fn a_new_event_retargets_the_running_effect() {
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let model = with_overlay(OverlayKind::Popup);
        let plain = draw(&mut terminal, &mut new(), &model, 0);
        let mut fx = new();
        fx.apply(vec![UiEvent::OverlayOpened(OverlayKind::Popup)]);
        draw(&mut terminal, &mut fx, &model, 0);
        let half = draw(&mut terminal, &mut fx, &model, 80);
        assert_ne!(half, plain);
        // The close replaces the open. tachyonfx runs the replaced effect
        // for one more frame, then drops it. At its start the slide out
        // shows the whole popup.
        fx.apply(vec![UiEvent::OverlayClosed(OverlayKind::Popup)]);
        draw(&mut terminal, &mut fx, &model, 0);
        let closing = draw(&mut terminal, &mut fx, &model, 0);
        assert_eq!(closing, plain);
    }

    /// Snapshots at 0, 80 and 160 ms.
    fn steps(
        model: &FrameModel,
        events: &[UiEvent],
        warmup: &FrameModel,
    ) -> (String, String, String) {
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let mut fx = new();
        draw(&mut terminal, &mut fx, warmup, 0);
        fx.apply(events.to_vec());
        let start = draw(&mut terminal, &mut fx, model, 0);
        let middle = draw(&mut terminal, &mut fx, model, 80);
        let end = draw(&mut terminal, &mut fx, model, 80);
        assert!(!fx.active());
        (start, middle, end)
    }

    /// Draws one frame: the tab bar and the list, then the effects.
    fn draw_tabs(
        terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>,
        transitions: &mut Transitions,
        model: &FrameModel,
        elapsed_ms: u64,
    ) -> Buffer {
        let areas = testdata::areas(WIDTH, HEIGHT);
        let theme = testdata::pierre();
        transitions.prepare(model, &areas);
        terminal
            .draw(|frame| {
                tabbar::render(
                    &model.tab_bar,
                    rect(areas.tab_bar),
                    frame.buffer_mut(),
                    &theme,
                    &mut HitMap::default(),
                );
                list::render(
                    &model.list,
                    rect(areas.list),
                    frame.buffer_mut(),
                    &theme,
                    &mut HitMap::default(),
                );
                transitions.process(frame, Duration::from_millis(elapsed_ms));
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// The cells `[l, r)` of each tab in the tab bar.
    fn tab_spans(model: &FrameModel) -> Vec<(u16, u16)> {
        let area = rect(testdata::areas(WIDTH, HEIGHT).tab_bar);
        let mut hits = HitMap::default();
        tabbar::render(
            &model.tab_bar,
            area,
            &mut Buffer::empty(area),
            &testdata::pierre(),
            &mut hits,
        );
        (0..model.tab_bar.titles.len())
            .map(|i| {
                let r = hits.region(Hit::Tab(i)).unwrap();
                (r.x, r.x + r.width)
            })
            .collect()
    }

    /// The cells `[l, r)` of the tabs that have the active tab background.
    fn highlight(buf: &Buffer, tabs: &[(u16, u16)]) -> (u16, u16) {
        let (bg, _) = testdata::pierre().active_tab_colors();
        let y = rect(testdata::areas(WIDTH, HEIGHT).tab_bar).y;
        let end = tabs.last().map_or(0, |&(_, r)| r);
        let lit: Vec<u16> = (0..end).filter(|&x| buf[(x, y)].bg == bg).collect();
        (lit[0], lit[lit.len() - 1] + 1)
    }

    /// The text of the list rows.
    fn list_text(buf: &Buffer) -> Vec<String> {
        let list = rect(testdata::areas(WIDTH, HEIGHT).list);
        (list.top()..list.bottom())
            .map(|y| {
                (list.left()..list.right())
                    .map(|x| buf[(x, y)].symbol())
                    .collect()
            })
            .collect()
    }

    /// A frame with three tabs and `active` as the active tab.
    fn three_tabs(active: usize) -> FrameModel {
        let mut model = testdata::sliding_frame();
        model.tab_bar = TabBarModel {
            titles: vec!["mine", "team", "repos"],
            active,
        };
        model
    }

    #[test]
    fn tab_slide() {
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let model = testdata::sliding_frame();
        let tabs = tab_spans(&model);
        let plain = draw_tabs(&mut terminal, &mut Transitions::new(false, 160), &model, 0);
        let mut fx = new();
        draw_tabs(&mut terminal, &mut fx, &testdata::frame(), 0);
        fx.apply(vec![UiEvent::TabChanged { from: 0, to: 1 }]);
        let start = draw_tabs(&mut terminal, &mut fx, &model, 0);
        assert_eq!(highlight(&start, &tabs), tabs[0], "on the old tab");
        assert_eq!(
            list_text(&start),
            list_text(&plain),
            "the list shows the new tab at once"
        );
        let middle = draw_tabs(&mut terminal, &mut fx, &model, 60);
        let (l, r) = highlight(&middle, &tabs);
        assert!(tabs[0].0 < l && l < tabs[1].0, "{l} between the tabs");
        assert!(tabs[0].1 < r && r < tabs[1].1, "{r} between the tabs");
        assert_eq!(list_text(&middle), list_text(&plain));
        let end = draw_tabs(&mut terminal, &mut fx, &model, 60);
        assert_eq!(highlight(&end, &tabs), tabs[1], "on the new tab");
        draw_tabs(&mut terminal, &mut fx, &model, 16);
        assert!(!fx.active());
    }

    #[test]
    fn tab_slide_off_shows_the_new_tab_at_once() {
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let model = testdata::sliding_frame();
        let tabs = tab_spans(&model);
        let mut fx = Transitions::new(false, 160).with_theme(testdata::pierre());
        fx.apply(vec![UiEvent::TabChanged { from: 0, to: 1 }]);
        assert!(!fx.active());
        let buf = draw_tabs(&mut terminal, &mut fx, &model, 0);
        assert_eq!(highlight(&buf, &tabs), tabs[1]);
    }

    #[test]
    fn a_second_tab_switch_retargets_the_tab_slide() {
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let tabs = tab_spans(&three_tabs(0));
        let mut fx = new();
        draw_tabs(&mut terminal, &mut fx, &three_tabs(0), 0);
        fx.apply(vec![UiEvent::TabChanged { from: 0, to: 1 }]);
        draw_tabs(&mut terminal, &mut fx, &three_tabs(1), 0);
        draw_tabs(&mut terminal, &mut fx, &three_tabs(1), 60);
        fx.apply(vec![UiEvent::TabChanged { from: 1, to: 2 }]);
        let model = three_tabs(2);
        draw_tabs(&mut terminal, &mut fx, &model, 0);
        let start = draw_tabs(&mut terminal, &mut fx, &model, 0);
        assert_eq!(highlight(&start, &tabs), tabs[1], "from the second tab");
        let middle = draw_tabs(&mut terminal, &mut fx, &model, 60);
        let (l, r) = highlight(&middle, &tabs);
        assert!(tabs[1].0 < l && l < tabs[2].0, "{l} between the tabs");
        assert!(tabs[1].1 < r && r < tabs[2].1, "{r} between the tabs");
        assert!(fx.active(), "the new glide runs its full time");
        let end = draw_tabs(&mut terminal, &mut fx, &model, 60);
        assert_eq!(highlight(&end, &tabs), tabs[2], "on the third tab");
        draw_tabs(&mut terminal, &mut fx, &model, 16);
        assert!(!fx.active());
    }

    #[test]
    fn popup_slide_in() {
        let model = with_overlay(OverlayKind::Popup);
        let (start, middle, end) = steps(
            &model,
            &[UiEvent::OverlayOpened(OverlayKind::Popup)],
            &testdata::frame(),
        );
        insta::assert_snapshot!("popup_in_0ms", start);
        insta::assert_snapshot!("popup_in_80ms", middle);
        insta::assert_snapshot!("popup_in_160ms", end);
    }

    #[test]
    fn popup_slide_out() {
        let mut model = with_overlay(OverlayKind::Popup);
        model.overlays[0].closing = true;
        let (start, middle, end) = steps(
            &model,
            &[UiEvent::OverlayClosed(OverlayKind::Popup)],
            &with_overlay(OverlayKind::Popup),
        );
        insta::assert_snapshot!("popup_out_0ms", start);
        insta::assert_snapshot!("popup_out_80ms", middle);
        insta::assert_snapshot!("popup_out_160ms", end);
    }

    #[test]
    fn help_slide_in() {
        let model = with_overlay(OverlayKind::Help);
        let (start, middle, end) = steps(
            &model,
            &[UiEvent::OverlayOpened(OverlayKind::Help)],
            &testdata::frame(),
        );
        insta::assert_snapshot!("help_in_0ms", start);
        insta::assert_snapshot!("help_in_80ms", middle);
        insta::assert_snapshot!("help_in_160ms", end);
    }

    #[test]
    fn scroll() {
        let mut model = testdata::frame();
        model.list.rows.extend(testdata::mine_list().rows);
        model.list.rows.extend(testdata::mine_list().rows);
        let warmup = model.clone();
        model.list.scroll = 4;
        let (start, middle, end) = steps(
            &model,
            &[UiEvent::ScrollTo {
                tab: "mine",
                from: 0,
                to: 4,
            }],
            &warmup,
        );
        insta::assert_snapshot!("scroll_0ms", start);
        insta::assert_snapshot!("scroll_80ms", middle);
        insta::assert_snapshot!("scroll_160ms", end);
    }

    #[test]
    fn status_fades_out() {
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let mut fx = new();
        let with_message = testdata::frame();
        let mut expired = testdata::frame();
        expired.status.center = None;
        draw(&mut terminal, &mut fx, &with_message, 0);
        fx.apply(vec![UiEvent::StatusExpired]);
        assert!(fx.active());
        let message = rect(testdata::areas(WIDTH, HEIGHT).status);
        let at = (message.x + 40, message.y);
        draw(&mut terminal, &mut fx, &expired, 0);
        assert_eq!(terminal.backend().buffer()[at].symbol(), "m");
        draw(&mut terminal, &mut fx, &expired, 150);
        let mid = terminal.backend().buffer()[at].fg;
        assert_ne!(mid, Color::Reset);
        assert_ne!(mid, testdata::pierre().palette.surface);
        draw(&mut terminal, &mut fx, &expired, 150);
        assert_eq!(
            terminal.backend().buffer()[at].fg,
            testdata::pierre().palette.surface
        );
        assert!(!fx.active());
        draw(&mut terminal, &mut fx, &expired, 16);
        assert_eq!(terminal.backend().buffer()[at].symbol(), " ");
    }
}
