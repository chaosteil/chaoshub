//! Transitions on tachyonfx. `update` pushes plain `UiEvent`s, and
//! `Transitions` turns them into effects that run over the frame buffer
//! after `view`.
//!
//! - Tab switch (`FxKey::Tab`): the highlight of the active tab glides
//!   from the old tab to the new tab in the tab bar row. The list shows
//!   the new tab at once.
//! - Pane tab switch (`FxKey::PaneTab`): the same glide on the pane tab
//!   row of the popup. When the two tabs are on different rows, the new
//!   tab shows at once. The pane content switches at once.
//! - Confirm, prompt and picker: `slide_in` from the bottom edge on open,
//!   `slide_out` on close.
//! - Popup, output and help: `slide_in` from the right edge, `slide_out`
//!   to the right.
//! - Scroll (`FxKey::Scroll`): the list rows move from the old offset to
//!   the new one.
//! - Cursor move (`FxKey::Band`): in the list, a pane table and the
//!   picker, the selection band wipes onto the new row from the left in
//!   60 ms. The old row loses its band at once. Not on the first frame of
//!   a list, a pane or a picker.
//! - New status message (`FxKey::StatusIn`): the message text slides in
//!   from the left edge of the message area of the status bar in 80 ms.
//! - Expired status message: `fade_to_fg` toward the status bar grey.
//! - Action success (`FxKey::Flash`): the list row of the pull request
//!   fades from the primary color to its background in 200 ms. Not when
//!   the row is out of view or under an overlay.
//! - Sync change (`FxKey::Glow(row)`): a visible row whose CI state, check
//!   counts or review counts changed fades from the surface color to its
//!   background in 250 ms. One glow per row at a time.

use std::rc::Rc;
use std::time::Duration;

use ratatui::Frame;
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::{Offset, Rect};
use ratatui::style::Color;
use tachyonfx::{
    EffectManager, EffectTimer, Interpolatable, Interpolation, Motion, RefCount, RefRect, fx,
    ref_count,
};

use crate::app::layout::Areas;
use crate::app::models::{FrameModel, OverlayContent, PaneModel, RowItem, TabBarModel};
use crate::bus::{Hit, OverlayKind, UiEvent};
use crate::domain::keys::PrKey;
use crate::theme::{Skin, Theme, ThemeKind};
use crate::ui::hits::HitMap;
use crate::ui::list::Window;
use crate::ui::status::message_place;
use crate::ui::{list, list_areas, overlay_area, picker, rect, render_overlay, screen, tabbar};

/// The duration of the confirm and prompt slides, in milliseconds.
const DIALOG_MS: u64 = 100;
/// The duration of the status fade, in milliseconds.
const STATUS_FADE_MS: u64 = 300;
/// The duration of the tab glide and the pane tab glide, in
/// milliseconds.
const TAB_GLIDE_MS: u64 = 120;
/// The duration of the band wipe, in milliseconds.
const BAND_WIPE_MS: u64 = 60;
/// The duration of the row flash after an action, in milliseconds.
const ROW_FLASH_MS: u64 = 200;
/// The duration of the row glow after a sync, in milliseconds.
const ROW_GLOW_MS: u64 = 250;
/// The duration of the slide in of a new status message, in
/// milliseconds.
const STATUS_IN_MS: u64 = 80;
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
    PaneTab,
    Band,
    Flash,
    /// The glow of the list row at this index.
    Glow(usize),
    Scroll,
    Status,
    StatusIn,
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
        UiEvent::PaneTabChanged { .. } => FxKey::PaneTab,
        UiEvent::RowFlash(_) => FxKey::Flash,
        // `prepare` keys a glow by its row.
        UiEvent::RowGlow(_) => FxKey::Glow(0),
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

/// What the cursor moves in: the list of a tab, the table pane of the
/// popup, or the picker. A band wipe runs only when the cursor moves
/// inside the same view, so the first frame of a view has none.
#[derive(Debug, Clone, PartialEq, Eq)]
enum CursorView {
    List { tab: usize, filter: Option<String> },
    Pane { title: String, tab: usize },
    Picker { title: String },
}

/// The view of the cursor in a frame and the cursor index. `None` when
/// the top overlay has no band, for example the help.
fn cursor_view(model: &FrameModel) -> Option<(CursorView, usize)> {
    let Some(overlay) = model.overlays.iter().rev().find(|o| !o.closing) else {
        let view = CursorView::List {
            tab: model.tab_bar.active,
            filter: model.filter.as_ref().map(|f| f.text.clone()),
        };
        return Some((view, model.list.cursor));
    };
    match &overlay.content {
        OverlayContent::Popup {
            title,
            active,
            pane: PaneModel::Table { .. },
            cursor,
            ..
        } => Some((
            CursorView::Pane {
                title: title.clone(),
                tab: *active,
            },
            *cursor,
        )),
        OverlayContent::Picker { title, cursor, .. } => Some((
            CursorView::Picker {
                title: title.clone(),
            },
            *cursor,
        )),
        _ => None,
    }
}

/// The state of a tab glide: in the tab bar or on a pane tab row.
#[derive(Debug, Clone, Default)]
struct TabGlide {
    /// The row of tabs with no active tab.
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
    /// The old and the new active tab of a tab glide that `prepare`
    /// starts.
    pending_tab: Option<(usize, usize)>,
    /// The old and the new pane (registry indexes) of a pane tab glide
    /// that `prepare` starts.
    pending_pane: Option<(usize, usize)>,
    /// The view and the cursor of the last `prepare`.
    last_cursor: Option<(CursorView, usize)>,
    /// The pull request of a row flash that `prepare` starts.
    pending_flash: Option<PrKey>,
    /// The pull requests of the row glows that `prepare` starts.
    pending_glows: Vec<PrKey>,
    /// The status message of the last `prepare`.
    last_message: Option<String>,
    /// True after the first `prepare`: the message of the first frame
    /// does not slide in.
    prepared: bool,
    /// The cells of a new status message that `process` slides in: from
    /// the left edge of the message area to the end of the message.
    message_in: Option<Rect>,
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
            pending_tab: None,
            pending_pane: None,
            last_cursor: None,
            pending_flash: None,
            pending_glows: Vec::new(),
            last_message: None,
            prepared: false,
            message_in: None,
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
                UiEvent::TabChanged { from, to } => self.pending_tab = Some((from, to)),
                UiEvent::PaneTabChanged { from, to } => self.pending_pane = Some((from, to)),
                UiEvent::RowFlash(pr) => self.pending_flash = Some(pr),
                UiEvent::RowGlow(pr) => self.pending_glows.push(pr),
                UiEvent::OverlayOpened(kind) => self.overlay_slide(key, kind, true),
                UiEvent::OverlayClosed(kind) => self.overlay_slide(key, kind, false),
                UiEvent::ScrollTo { from, to, .. } => self.scroll(key, from, to),
                UiEvent::StatusExpired => self.status_pending = true,
            }
        }
    }

    /// Starts a glide: the highlight of the active tab moves from the old
    /// tab to the new tab over the row of `glide`. Each frame, the effect
    /// copies the row with no active tab over the frame and gives the
    /// cells of the highlight the active tab colors. The tab bar and the
    /// pane tabs share it.
    fn glide(&mut self, key: FxKey, glide: TabGlide) {
        let (bg, fg) = self.theme.active_tab_colors();
        let timer = Self::timer(Duration::from_millis(TAB_GLIDE_MS));
        let effect = fx::effect_fn_buf(glide, timer, move |glide, cx, buf| {
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

    /// The glide of the tab bar: the bar with no active tab, and the
    /// spans of the old and the new tab from the hit regions.
    fn tab_glide(
        &self,
        model: &FrameModel,
        areas: &Areas,
        from: usize,
        to: usize,
    ) -> Option<TabGlide> {
        let bar_area = rect(areas.tab_bar);
        let mut bar = Buffer::empty(bar_area);
        let mut hits = HitMap::default();
        let idle = TabBarModel {
            titles: model.tab_bar.titles.clone(),
            active: model.tab_bar.titles.len(),
        };
        tabbar::render(&idle, bar_area, &mut bar, &self.theme, &mut hits);
        let span = |i| hits.region(Hit::Tab(i)).map(|r| (r.x, r.width));
        Some(TabGlide {
            bar,
            from: span(from)?,
            to: span(to)?,
        })
    }

    /// The glide of the pane tabs of the open popup: the popup drawn with
    /// no active tab, and the spans of the old and the new pane tab from
    /// the hit regions. The row holds the two tabs. `None` when the two
    /// tabs are on different rows.
    fn pane_glide(
        &self,
        model: &FrameModel,
        areas: &Areas,
        from: usize,
        to: usize,
    ) -> Option<TabGlide> {
        let mut overlay = model
            .overlays
            .iter()
            .rev()
            .find(|o| !o.closing && matches!(o.content, OverlayContent::Popup { .. }))?
            .clone();
        let OverlayContent::Popup { tabs, active, .. } = &mut overlay.content else {
            return None;
        };
        let position = |pane: usize| tabs.iter().position(|tab| tab.pane == pane);
        let (from, to) = (position(from)?, position(to)?);
        // The skins draw the active tab narrower than its hit region:
        // spaces fill the rest of the region.
        let drawn = |i: usize| {
            let width: usize = self
                .theme
                .tab(&tabs[i].label(), true)
                .iter()
                .map(ratatui::text::Span::width)
                .sum();
            u16::try_from(width).unwrap_or(u16::MAX)
        };
        let (from_width, to_width) = (drawn(from), drawn(to));
        *active = usize::MAX;
        let mut buf = Buffer::empty(screen(areas));
        let mut hits = HitMap::default();
        render_overlay(&overlay, areas, &mut buf, &self.theme, &mut hits);
        let old = hits.region(Hit::PaneTab(from))?;
        let new = hits.region(Hit::PaneTab(to))?;
        if old.y != new.y {
            return None;
        }
        let left = old.x.min(new.x);
        let right = old.right().max(new.right());
        Some(TabGlide {
            bar: copy(&buf, Rect::new(left, old.y, right - left, 1)),
            from: (old.x, old.width.min(from_width)),
            to: (new.x, new.width.min(to_width)),
        })
    }

    /// The band wipes onto the cursor row: at alpha `a`, the cells left of
    /// `a × width` have the band (the frame), and the others the row
    /// without the band (`plain`).
    fn band_wipe(&mut self, plain: Buffer) {
        let timer = Self::timer(Duration::from_millis(BAND_WIPE_MS));
        let effect = fx::effect_fn_buf(plain, timer, |plain, cx, buf| {
            let area = plain.area;
            let cut = (f32::from(area.width) * cx.alpha().clamp(0.0, 1.0)).round();
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "cut is rounded and between 0 and the width"
            )]
            let cut = (cut as u16).min(area.width);
            let rest = Rect::new(area.x + cut, area.y, area.width - cut, area.height);
            for (x, y) in positions(rest) {
                if let Some(dst) = buf.cell_mut((x, y)) {
                    *dst = plain[(x, y)].clone();
                }
            }
        });
        self.manager.add_unique_effect(FxKey::Band, effect);
    }

    /// The cursor row of `view` drawn without the band, at its place on
    /// the screen. `None` when the row is not drawn.
    fn plain_row(
        &self,
        model: &FrameModel,
        areas: &Areas,
        view: &CursorView,
        cursor: usize,
    ) -> Option<Buffer> {
        let mut buf = Buffer::empty(screen(areas));
        let mut hits = HitMap::default();
        let overlay = || model.overlays.iter().rev().find(|o| !o.closing).cloned();
        let row = match view {
            CursorView::List { .. } => {
                let window = Window {
                    cursor: None,
                    scroll: model.list.scroll,
                    hit: Hit::Row,
                };
                let body = list_areas(model, areas).body;
                let list = &model.list;
                list::render_rows(
                    &list.columns,
                    &list.rows,
                    window,
                    body,
                    &mut buf,
                    &self.theme,
                    &mut hits,
                );
                hits.region(Hit::Row(cursor))?
            }
            CursorView::Pane { .. } => {
                let mut overlay = overlay()?;
                if let OverlayContent::Popup { cursor, .. } = &mut overlay.content {
                    *cursor = usize::MAX;
                }
                render_overlay(&overlay, areas, &mut buf, &self.theme, &mut hits);
                hits.region(Hit::PaneRow(cursor))?
            }
            CursorView::Picker { .. } => {
                let overlay = overlay()?;
                let OverlayContent::Picker { items, .. } = &overlay.content else {
                    return None;
                };
                render_overlay(&overlay, areas, &mut buf, &self.theme, &mut hits);
                let line = hits.region(Hit::Item(cursor))?;
                let p = &self.theme.palette;
                buf.set_style(line, ratatui::style::Style::new().bg(p.bg).fg(p.fg));
                for (x, y) in positions(line) {
                    buf[(x, y)].set_symbol(" ");
                }
                picker::plain_item(items.get(cursor)?, line, &mut buf, &self.theme);
                line
            }
        };
        Some(copy(&buf, row))
    }

    /// The background of the cells of `row` fades from `from` to the
    /// background of the frame in `ms`. The text keeps its color, and a
    /// row with the band fades toward the band.
    fn row_fade(&mut self, key: FxKey, row: Rect, from: Color, ms: u64) {
        let behind = self.theme.palette.bg;
        let timer = Self::timer(Duration::from_millis(ms));
        let effect = fx::effect_fn_buf(row, timer, move |row, cx, buf| {
            let alpha = cx.alpha().clamp(0.0, 1.0);
            for (x, y) in positions(*row) {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    let target = if cell.bg == Color::Reset {
                        behind
                    } else {
                        cell.bg
                    };
                    cell.set_bg(from.lerp(&target, alpha));
                }
            }
        });
        self.manager.add_unique_effect(key, effect);
    }

    /// The index and the screen row of the list row of `pr`. `None` when
    /// the row is out of view, or when an overlay is open over the list.
    fn list_row(model: &FrameModel, areas: &Areas, pr: &PrKey) -> Option<(usize, Rect)> {
        if model.overlays.iter().any(|o| !o.closing) {
            return None;
        }
        let list = &model.list;
        let index = list
            .rows
            .iter()
            .position(|row| matches!(&row.item, RowItem::Pr(r) if r.key == *pr))?;
        let body = list_areas(model, areas).body;
        let offset = index.checked_sub(list.scroll)?;
        let offset = u16::try_from(offset).ok().filter(|&o| o < body.height)?;
        Some((index, Rect::new(body.x, body.y + offset, body.width, 1)))
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

    /// Starts the pending tab glides, draws the scroll rows into the
    /// scratch buffer, and updates the areas of the running effects.
    pub(crate) fn prepare(&mut self, model: &FrameModel, areas: &Areas) {
        self.areas = Some(*areas);
        if !self.enabled {
            return;
        }
        let list_area = list_areas(model, areas).body;
        self.list.set(list_area);
        for overlay in &model.overlays {
            let area = effect_area(overlay_area(overlay, areas), areas, &self.theme);
            self.overlays[slot(overlay.kind)].set(area);
        }
        if let Some((from, to)) = self.pending_tab.take()
            && let Some(glide) = self.tab_glide(model, areas, from, to)
        {
            self.glide(FxKey::Tab, glide);
        }
        if let Some(pr) = self.pending_flash.take()
            && let Some((_, row)) = Self::list_row(model, areas, &pr)
        {
            self.row_fade(FxKey::Flash, row, self.theme.palette.primary, ROW_FLASH_MS);
        }
        for pr in std::mem::take(&mut self.pending_glows) {
            if let Some((index, row)) = Self::list_row(model, areas, &pr) {
                let surface = self.theme.palette.surface;
                self.row_fade(FxKey::Glow(index), row, surface, ROW_GLOW_MS);
            }
        }
        let message = model.status.center.as_ref().map(|c| c.text.clone());
        if self.prepared && message.is_some() && self.last_message != message {
            self.message_in =
                message_place(&model.status, rect(areas.status), &self.theme).map(|place| {
                    let width = crate::ui::text::width(&place.text);
                    let end = place.x + u16::try_from(width).unwrap_or(u16::MAX);
                    Rect::new(place.area.x, place.area.y, end - place.area.x, 1)
                });
        }
        self.last_message = message;
        self.prepared = true;
        let current = cursor_view(model);
        if let (Some((view, cursor)), Some((last_view, last))) = (&current, &self.last_cursor)
            && view == last_view
            && cursor != last
            && let Some(plain) = self.plain_row(model, areas, view, *cursor)
        {
            self.band_wipe(plain);
        }
        self.last_cursor = current;
        if let Some((from, to)) = self.pending_pane.take() {
            match self.pane_glide(model, areas, from, to) {
                Some(glide) => self.glide(FxKey::PaneTab, glide),
                // Tabs on two rows: no glide, and a running one stops, so
                // the new tab shows at once.
                None => {
                    self.manager.cancel_unique_effect(FxKey::PaneTab);
                }
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
            if let Some(span) = self.message_in.take() {
                self.status_slide_in(buf, span.intersection(status));
            }
            self.status_row = Some(copy(buf, status));
        }
        self.manager.process_effects(elapsed.into(), buf, area);
    }

    /// Starts the slide in of a new status message. `span` runs from the
    /// left edge of the message area to the end of the message. At alpha
    /// `a`, the cells of the span show the text moved left by
    /// `(1 − a) × width`, and the cells right of the text are blank.
    fn status_slide_in(&mut self, buf: &Buffer, span: Rect) {
        if span.is_empty() {
            return;
        }
        let cells = copy(buf, span);
        let mut blank = ratatui::buffer::Cell::default();
        blank
            .set_symbol(" ")
            .set_style(self.theme.status_band().bar);
        let timer = Self::timer(Duration::from_millis(STATUS_IN_MS));
        let effect = fx::effect_fn_buf(cells, timer, move |cells, cx, buf| {
            let area = cells.area;
            let shift = (f32::from(area.width) * (1.0 - cx.alpha().clamp(0.0, 1.0))).round();
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "shift is rounded and between 0 and the width"
            )]
            let shift = shift as u16;
            for dx in 0..area.width {
                let source = dx + shift;
                let cell = if source < area.width {
                    cells[(area.x + source, area.y)].clone()
                } else {
                    blank.clone()
                };
                if let Some(dst) = buf.cell_mut((area.x + dx, area.y)) {
                    *dst = cell;
                }
            }
        });
        self.manager.add_unique_effect(FxKey::StatusIn, effect);
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
        self.pending_tab = None;
        self.pending_pane = None;
        self.pending_flash = None;
        self.pending_glows.clear();
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

/// The area of the slide effect of an overlay in `area`. The punk skin
/// draws a drop shadow one column right of the overlay and one row below
/// it, so the effect also covers that column and that row, clipped to
/// the screen and to the rows above the key hint bar. The shadow then
/// slides with the overlay. Other skins keep the overlay area.
fn effect_area(area: Rect, areas: &Areas, theme: &Theme) -> Rect {
    if theme.skin != Skin::Punk || area.is_empty() {
        return area;
    }
    let bars = if areas.hints.height > 0 {
        areas.hints.y
    } else {
        areas.status.y
    };
    let screen = Rect::new(0, 0, areas.status.width, bars);
    Rect::new(area.x, area.y, area.width + 1, area.height + 1).intersection(screen)
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

    /// Draws one full frame with `view`, then the effects.
    fn draw_view(
        terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>,
        transitions: &mut Transitions,
        model: &FrameModel,
        elapsed_ms: u64,
    ) -> Buffer {
        draw_view_in(
            terminal,
            transitions,
            model,
            elapsed_ms,
            &testdata::pierre(),
        )
    }

    /// Draws one full frame with `view` in `theme`, then the effects.
    fn draw_view_in(
        terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>,
        transitions: &mut Transitions,
        model: &FrameModel,
        elapsed_ms: u64,
        theme: &Theme,
    ) -> Buffer {
        let areas = testdata::areas(WIDTH, HEIGHT);
        transitions.prepare(model, &areas);
        terminal
            .draw(|frame| {
                crate::ui::view(frame, model, &areas, theme, &mut HitMap::default());
                transitions.process(frame, Duration::from_millis(elapsed_ms));
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// The popup frame with the pane tabs of the popup fixture and of the
    /// panes `extra`, and the tab at `active` (a position) active.
    fn popup_frame(active: usize, extra: &[(char, &'static str)]) -> FrameModel {
        let mut overlay = testdata::popup_overlay();
        if let OverlayContent::Popup {
            tabs, active: a, ..
        } = &mut overlay.content
        {
            for &(key, title) in extra {
                let pane = tabs.len();
                tabs.push(crate::app::models::PaneTab { key, title, pane });
            }
            *a = active;
        }
        testdata::with_overlay(overlay)
    }

    /// The cells `[l, r)` and the row of each pane tab of the popup.
    fn pane_spans(model: &FrameModel) -> Vec<(u16, u16, u16)> {
        let areas = testdata::areas(WIDTH, HEIGHT);
        let mut buf = Buffer::empty(screen(&areas));
        let mut hits = HitMap::default();
        render_overlay(
            &model.overlays[0],
            &areas,
            &mut buf,
            &testdata::pierre(),
            &mut hits,
        );
        let OverlayContent::Popup { tabs, .. } = &model.overlays[0].content else {
            unreachable!()
        };
        // The highlight of an active tab: its drawn cells, without the
        // spaces that fill the hit region.
        (0..tabs.len())
            .map(|i| {
                let r = hits.region(Hit::PaneTab(i)).unwrap();
                let drawn: usize = testdata::pierre()
                    .tab(&tabs[i].label(), true)
                    .iter()
                    .map(ratatui::text::Span::width)
                    .sum();
                (r.x, r.x + u16::try_from(drawn).unwrap(), r.y)
            })
            .collect()
    }

    /// The cells `[l, r)` of the pane tab row `y` that have the active tab
    /// background.
    fn pane_highlight(buf: &Buffer, tabs: &[(u16, u16, u16)], y: u16) -> (u16, u16) {
        let (bg, _) = testdata::pierre().active_tab_colors();
        let row: Vec<_> = tabs.iter().filter(|t| t.2 == y).collect();
        let (start, end) = (row[0].0, row[row.len() - 1].1);
        let lit: Vec<u16> = (start..end).filter(|&x| buf[(x, y)].bg == bg).collect();
        (lit[0], lit[lit.len() - 1] + 1)
    }

    #[test]
    fn pane_tab_slide() {
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let model = popup_frame(2, &[]);
        let tabs = pane_spans(&model);
        let y = tabs[0].2;
        let span = |i: usize| (tabs[i].0, tabs[i].1);
        let plain = draw_view(&mut terminal, &mut Transitions::new(false, 160), &model, 0);
        let mut fx = new();
        draw_view(&mut terminal, &mut fx, &popup_frame(1, &[]), 0);
        fx.apply(vec![UiEvent::PaneTabChanged { from: 1, to: 2 }]);
        let start = draw_view(&mut terminal, &mut fx, &model, 0);
        assert!(fx.active());
        assert_eq!(pane_highlight(&start, &tabs, y), span(1), "on the old tab");
        let body = |buf: &Buffer| -> Vec<String> {
            (y + 2..HEIGHT - 2)
                .map(|row| (0..WIDTH).map(|x| buf[(x, row)].symbol()).collect())
                .collect()
        };
        assert_eq!(body(&start), body(&plain), "the pane switches at once");
        let middle = draw_view(&mut terminal, &mut fx, &model, 60);
        let (l, r) = pane_highlight(&middle, &tabs, y);
        assert!(span(1).0 < l && l < span(2).0, "{l} between the tabs");
        assert!(span(1).1 < r && r < span(2).1, "{r} between the tabs");
        let end = draw_view(&mut terminal, &mut fx, &model, 60);
        assert_eq!(pane_highlight(&end, &tabs, y), span(2), "on the new tab");
        draw_view(&mut terminal, &mut fx, &model, 16);
        assert!(!fx.active());
    }

    #[test]
    fn a_second_pane_tab_switch_retargets_the_glide() {
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let tabs = pane_spans(&popup_frame(0, &[]));
        let y = tabs[0].2;
        let span = |i: usize| (tabs[i].0, tabs[i].1);
        let mut fx = new();
        draw_view(&mut terminal, &mut fx, &popup_frame(0, &[]), 0);
        fx.apply(vec![UiEvent::PaneTabChanged { from: 0, to: 1 }]);
        draw_view(&mut terminal, &mut fx, &popup_frame(1, &[]), 0);
        draw_view(&mut terminal, &mut fx, &popup_frame(1, &[]), 60);
        fx.apply(vec![UiEvent::PaneTabChanged { from: 1, to: 3 }]);
        let model = popup_frame(3, &[]);
        draw_view(&mut terminal, &mut fx, &model, 0);
        let start = draw_view(&mut terminal, &mut fx, &model, 0);
        assert_eq!(
            pane_highlight(&start, &tabs, y),
            span(1),
            "from the second tab"
        );
        let middle = draw_view(&mut terminal, &mut fx, &model, 60);
        let (l, r) = pane_highlight(&middle, &tabs, y);
        assert!(span(1).0 < l && l < span(3).0, "{l} between the tabs");
        assert!(span(1).1 < r && r < span(3).1, "{r} between the tabs");
        let end = draw_view(&mut terminal, &mut fx, &model, 60);
        assert_eq!(pane_highlight(&end, &tabs, y), span(3), "on the fourth tab");
        draw_view(&mut terminal, &mut fx, &model, 16);
        assert!(!fx.active());
    }

    #[test]
    fn pane_tabs_on_two_rows_and_animations_off_switch_at_once() {
        let extra = [('5', "Files"), ('6', "Agent")];
        let model = popup_frame(5, &extra);
        let tabs = pane_spans(&model);
        assert_ne!(tabs[4].2, tabs[5].2, "Files and Agent are on two rows");
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let mut fx = new();
        draw_view(&mut terminal, &mut fx, &popup_frame(4, &extra), 0);
        fx.apply(vec![UiEvent::PaneTabChanged { from: 4, to: 5 }]);
        let buf = draw_view(&mut terminal, &mut fx, &model, 0);
        assert!(!fx.active(), "no glide across rows");
        assert_eq!(
            pane_highlight(&buf, &tabs, tabs[5].2),
            (tabs[5].0, tabs[5].1)
        );
        let model = popup_frame(2, &[]);
        let tabs = pane_spans(&model);
        let mut fx = Transitions::new(false, 160).with_theme(testdata::pierre());
        fx.apply(vec![UiEvent::PaneTabChanged { from: 1, to: 2 }]);
        let buf = draw_view(&mut terminal, &mut fx, &model, 0);
        assert!(!fx.active());
        assert_eq!(
            pane_highlight(&buf, &tabs, tabs[0].2),
            (tabs[2].0, tabs[2].1)
        );
    }

    #[test]
    fn the_punk_shadow_slides_with_the_overlay() {
        let theme = testdata::punk();
        let areas = testdata::areas(WIDTH, HEIGHT);
        let model = testdata::with_overlay(testdata::confirm_overlay(false));
        let area = overlay_area(&model.overlays[0], &areas);
        // A shadow cell of the right column and one of the row below.
        let cells = [(area.right(), area.y + 1), (area.x + 1, area.bottom())];
        let shadow = |buf: &Buffer| cells.map(|at| buf[at].bg == theme.palette.primary);
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let mut fx = Transitions::new(true, 160).with_theme(theme);
        draw_view_in(&mut terminal, &mut fx, &testdata::frame(), 0, &theme);
        fx.apply(vec![UiEvent::OverlayOpened(OverlayKind::Confirm)]);
        let start = draw_view_in(&mut terminal, &mut fx, &model, 0, &theme);
        assert_eq!(shadow(&start), [false, false], "no shadow before the slide");
        let end = draw_view_in(&mut terminal, &mut fx, &model, 100, &theme);
        draw_view_in(&mut terminal, &mut fx, &model, 16, &theme);
        assert!(!fx.active());
        assert_eq!(
            shadow(&end),
            [true, true],
            "the shadow is in place at the end"
        );
        assert_eq!(
            effect_area(area, &areas, &theme),
            Rect::new(area.x, area.y, area.width + 1, area.height + 1)
        );
        assert_eq!(effect_area(area, &areas, &testdata::pierre()), area);
        let popup = rect(areas.popup);
        assert_eq!(
            effect_area(popup, &areas, &theme),
            popup,
            "clipped to the screen and to the rows above the key hint bar"
        );
    }

    /// For each cell of the row `y` from `x0` to `x1`: true when it has
    /// the band background of the plain skins.
    fn band(buf: &Buffer, y: u16, x0: u16, x1: u16) -> Vec<bool> {
        let surface = testdata::pierre().palette.surface;
        (x0..x1).map(|x| buf[(x, y)].bg == surface).collect()
    }

    /// The mine list frame with the cursor on row `cursor`.
    fn list_at(cursor: usize) -> FrameModel {
        let mut model = testdata::frame();
        model.list.cursor = cursor;
        model
    }

    /// The screen row of the list row `index` (no scroll).
    fn list_y(index: usize) -> u16 {
        let model = testdata::frame();
        let body = list_areas(&model, &testdata::areas(WIDTH, HEIGHT)).body;
        body.y + u16::try_from(index).unwrap()
    }

    #[test]
    fn the_band_wipes_onto_the_new_list_row() {
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let mut fx = new();
        draw_view(&mut terminal, &mut fx, &list_at(1), 0);
        assert!(!fx.active(), "no wipe on the first frame");
        let start = draw_view(&mut terminal, &mut fx, &list_at(2), 0);
        assert!(fx.active());
        assert_eq!(band(&start, list_y(2), 0, WIDTH), vec![false; 100]);
        assert_eq!(
            band(&start, list_y(1), 0, WIDTH),
            vec![false; 100],
            "the old row loses its band at once"
        );
        let middle = draw_view(&mut terminal, &mut fx, &list_at(2), 30);
        let lit = band(&middle, list_y(2), 0, WIDTH);
        let cut = lit.iter().position(|&b| !b).unwrap();
        assert!(cut > 10 && cut < 99, "the band is part of the row: {cut}");
        assert!(lit[..cut].iter().all(|&b| b) && lit[cut..].iter().all(|&b| !b));
        let end = draw_view(&mut terminal, &mut fx, &list_at(2), 30);
        assert_eq!(band(&end, list_y(2), 0, WIDTH), vec![true; 100]);
        draw_view(&mut terminal, &mut fx, &list_at(2), 16);
        assert!(!fx.active());
    }

    #[test]
    fn a_second_move_retargets_the_band_wipe() {
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let mut fx = new();
        draw_view(&mut terminal, &mut fx, &list_at(1), 0);
        draw_view(&mut terminal, &mut fx, &list_at(2), 0);
        draw_view(&mut terminal, &mut fx, &list_at(2), 30);
        draw_view(&mut terminal, &mut fx, &list_at(3), 0);
        let start = draw_view(&mut terminal, &mut fx, &list_at(3), 0);
        assert_eq!(band(&start, list_y(3), 0, WIDTH), vec![false; 100]);
        assert_eq!(band(&start, list_y(2), 0, WIDTH), vec![false; 100]);
        let end = draw_view(&mut terminal, &mut fx, &list_at(3), 60);
        assert_eq!(band(&end, list_y(3), 0, WIDTH), vec![true; 100]);
    }

    #[test]
    fn no_band_wipe_on_a_new_list_or_with_animations_off() {
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let mut fx = new();
        draw_view(&mut terminal, &mut fx, &list_at(1), 0);
        let mut other = testdata::sliding_frame();
        other.list.cursor = 3;
        draw_view(&mut terminal, &mut fx, &other, 0);
        assert!(!fx.active(), "a tab switch is a new list");
        let mut filtered = list_at(2);
        filtered.filter = Some(crate::app::models::FilterModel {
            text: "x".into(),
            editing: true,
        });
        draw_view(&mut terminal, &mut fx, &list_at(1), 0);
        draw_view(&mut terminal, &mut fx, &filtered, 0);
        assert!(!fx.active(), "a filter change is a new list");
        let mut fx = Transitions::new(false, 160).with_theme(testdata::pierre());
        draw_view(&mut terminal, &mut fx, &list_at(1), 0);
        let buf = draw_view(&mut terminal, &mut fx, &list_at(2), 0);
        assert!(!fx.active());
        assert_eq!(band(&buf, list_y(2), 0, WIDTH), vec![true; 100]);
    }

    #[test]
    fn the_band_wipes_in_a_pane_table_and_the_picker() {
        let areas = testdata::areas(WIDTH, HEIGHT);
        let popup = |cursor: usize| {
            let mut overlay = testdata::popup_overlay();
            if let OverlayContent::Popup { cursor: c, .. } = &mut overlay.content {
                *c = cursor;
            }
            testdata::with_overlay(overlay)
        };
        let picker = |cursor| testdata::with_overlay(testdata::picker_overlay(5, cursor));
        let row_of = |model: &FrameModel, hit: Hit| {
            let mut buf = Buffer::empty(screen(&areas));
            let mut hits = HitMap::default();
            render_overlay(
                &model.overlays[0],
                &areas,
                &mut buf,
                &testdata::pierre(),
                &mut hits,
            );
            hits.region(hit).unwrap()
        };
        let cases = [
            (popup(0), popup(1), row_of(&popup(1), Hit::PaneRow(1))),
            (picker(0), picker(2), row_of(&picker(2), Hit::Item(2))),
        ];
        for (before, after, row) in cases {
            let mut terminal = testdata::terminal(WIDTH, HEIGHT);
            let mut fx = new();
            draw_view(&mut terminal, &mut fx, &before, 0);
            let start = draw_view(&mut terminal, &mut fx, &after, 0);
            let width = usize::from(row.width);
            let (x0, x1) = (row.x, row.right());
            assert_eq!(band(&start, row.y, x0, x1), vec![false; width], "{row:?}");
            let end = draw_view(&mut terminal, &mut fx, &after, 60);
            assert_eq!(band(&end, row.y, x0, x1), vec![true; width], "{row:?}");
        }
    }

    /// The key of the pull request of the mine list row `index`.
    fn row_key(index: usize) -> PrKey {
        match &testdata::frame().list.rows[index].item {
            RowItem::Pr(pr) => pr.key.clone(),
            item => panic!("not a pull request row: {item:?}"),
        }
    }

    #[test]
    fn a_row_flashes_after_an_action() {
        let p = testdata::pierre().palette;
        let model = list_at(1);
        let bg = |buf: &Buffer, index: usize| buf[(10, list_y(index))].bg;
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let mut fx = new();
        let plain = draw_view(&mut terminal, &mut fx, &model, 0);
        fx.apply(vec![UiEvent::RowFlash(row_key(2))]);
        let start = draw_view(&mut terminal, &mut fx, &model, 0);
        assert!(fx.active());
        assert_eq!(bg(&start, 2), p.primary);
        assert_eq!(
            start[(10, list_y(2))].fg,
            plain[(10, list_y(2))].fg,
            "the text keeps its color"
        );
        let middle = draw_view(&mut terminal, &mut fx, &model, 100);
        assert_ne!(bg(&middle, 2), p.primary);
        assert_ne!(bg(&middle, 2), bg(&plain, 2));
        let end = draw_view(&mut terminal, &mut fx, &model, 100);
        assert_eq!(bg(&end, 2), bg(&plain, 2));
        draw_view(&mut terminal, &mut fx, &model, 16);
        assert!(!fx.active());
        // On the cursor row, the flash fades toward the band.
        fx.apply(vec![UiEvent::RowFlash(row_key(1))]);
        draw_view(&mut terminal, &mut fx, &model, 0);
        let end = draw_view(&mut terminal, &mut fx, &model, 200);
        assert_eq!(bg(&end, 1), p.surface);
    }

    #[test]
    fn a_second_flash_retargets_and_hidden_rows_do_not_flash() {
        let p = testdata::pierre().palette;
        let model = list_at(1);
        let bg = |buf: &Buffer, index: usize| buf[(10, list_y(index))].bg;
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let mut fx = new();
        let plain = draw_view(&mut terminal, &mut fx, &model, 0);
        fx.apply(vec![UiEvent::RowFlash(row_key(2))]);
        draw_view(&mut terminal, &mut fx, &model, 0);
        draw_view(&mut terminal, &mut fx, &model, 50);
        fx.apply(vec![UiEvent::RowFlash(row_key(3))]);
        draw_view(&mut terminal, &mut fx, &model, 0);
        let start = draw_view(&mut terminal, &mut fx, &model, 0);
        assert_eq!(bg(&start, 3), p.primary, "the new row flashes");
        assert_eq!(bg(&start, 2), bg(&plain, 2), "the old flash stops");
        let mut fx = new();
        let with_popup = testdata::with_overlay(testdata::popup_overlay());
        draw_view(&mut terminal, &mut fx, &with_popup, 0);
        fx.apply(vec![UiEvent::RowFlash(row_key(2))]);
        draw_view(&mut terminal, &mut fx, &with_popup, 0);
        assert!(!fx.active(), "the row is under the popup");
        let mut scrolled = list_at(4);
        scrolled.list.scroll = 3;
        draw_view(&mut terminal, &mut fx, &scrolled, 0);
        fx.apply(vec![UiEvent::RowFlash(row_key(2))]);
        draw_view(&mut terminal, &mut fx, &scrolled, 0);
        assert!(!fx.active(), "the row is above the window");
        let mut fx = Transitions::new(false, 160).with_theme(testdata::pierre());
        fx.apply(vec![UiEvent::RowFlash(row_key(2))]);
        let buf = draw_view(&mut terminal, &mut fx, &model, 0);
        assert!(!fx.active());
        assert_eq!(bg(&buf, 2), bg(&plain, 2), "nothing with animations off");
    }

    #[test]
    fn a_synced_row_glows_and_each_row_has_one_glow() {
        let p = testdata::pierre().palette;
        let model = list_at(1);
        let bg = |buf: &Buffer, index: usize| buf[(10, list_y(index))].bg;
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let mut fx = new();
        let plain = draw_view(&mut terminal, &mut fx, &model, 0);
        fx.apply(vec![
            UiEvent::RowGlow(row_key(2)),
            UiEvent::RowGlow(row_key(3)),
        ]);
        let start = draw_view(&mut terminal, &mut fx, &model, 0);
        assert!(fx.active());
        assert_eq!((bg(&start, 2), bg(&start, 3)), (p.surface, p.surface));
        let middle = draw_view(&mut terminal, &mut fx, &model, 125);
        assert_ne!(bg(&middle, 2), p.surface);
        assert_ne!(bg(&middle, 2), bg(&plain, 2));
        // A second glow of row 2 starts it again; row 3 runs on.
        fx.apply(vec![UiEvent::RowGlow(row_key(2))]);
        draw_view(&mut terminal, &mut fx, &model, 0);
        let again = draw_view(&mut terminal, &mut fx, &model, 0);
        assert_eq!(bg(&again, 2), p.surface, "the glow of row 2 restarts");
        assert_ne!(bg(&again, 3), p.surface, "row 3 keeps its own glow");
        let end = draw_view(&mut terminal, &mut fx, &model, 250);
        assert_eq!((bg(&end, 2), bg(&end, 3)), (bg(&plain, 2), bg(&plain, 3)));
        draw_view(&mut terminal, &mut fx, &model, 16);
        assert!(!fx.active());
        let mut fx = Transitions::new(false, 160).with_theme(testdata::pierre());
        fx.apply(vec![UiEvent::RowGlow(row_key(2))]);
        let buf = draw_view(&mut terminal, &mut fx, &model, 0);
        assert!(!fx.active());
        assert_eq!(bg(&buf, 2), bg(&plain, 2), "nothing with animations off");
    }

    /// The frame with the status message `text`.
    fn with_message(text: &str) -> FrameModel {
        let mut model = testdata::frame();
        model.status.center = Some(testdata::cell(text, crate::app::models::CellStyle::Success));
        model
    }

    /// The text of the status row.
    fn status_text(buf: &Buffer) -> String {
        let y = rect(testdata::areas(WIDTH, HEIGHT).status).y;
        (0..WIDTH).map(|x| buf[(x, y)].symbol()).collect()
    }

    #[test]
    fn a_new_status_message_slides_in_from_the_left() {
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let new_message = with_message("fetched 3 pull requests");
        let plain = status_text(&draw_view(
            &mut terminal,
            &mut Transitions::new(false, 160),
            &new_message,
            0,
        ));
        let at = plain.find("fetched").unwrap();
        let mut fx = new();
        let first = draw_view(&mut terminal, &mut fx, &with_message("merged #101"), 0);
        assert!(!fx.active(), "the first frame does not slide");
        assert!(status_text(&first).contains("merged #101"));
        let start = status_text(&draw_view(&mut terminal, &mut fx, &new_message, 0));
        assert!(fx.active());
        assert!(!start.contains("fetched"), "{start}");
        let middle = status_text(&draw_view(&mut terminal, &mut fx, &new_message, 40));
        let middle_at = middle.find("ests").unwrap();
        let final_at = plain.find("ests").unwrap();
        assert!(middle_at < final_at, "the text moves in from the left");
        assert!(!middle.contains("fetched"), "the start of the text is out");
        let end = status_text(&draw_view(&mut terminal, &mut fx, &new_message, 40));
        assert_eq!(end, plain);
        assert_eq!(end.find("fetched"), Some(at));
        draw_view(&mut terminal, &mut fx, &new_message, 16);
        assert!(!fx.active());
    }

    #[test]
    fn a_second_status_message_retargets_the_slide() {
        let mut terminal = testdata::terminal(WIDTH, HEIGHT);
        let third = with_message("approved #102");
        let plain = status_text(&draw_view(
            &mut terminal,
            &mut Transitions::new(false, 160),
            &third,
            0,
        ));
        let mut fx = new();
        draw_view(&mut terminal, &mut fx, &with_message("merged #101"), 0);
        draw_view(&mut terminal, &mut fx, &with_message("rebased"), 0);
        draw_view(&mut terminal, &mut fx, &with_message("rebased"), 40);
        draw_view(&mut terminal, &mut fx, &third, 0);
        let start = status_text(&draw_view(&mut terminal, &mut fx, &third, 0));
        assert!(
            !start.contains("approved") && !start.contains("rebased"),
            "{start}"
        );
        let end = status_text(&draw_view(&mut terminal, &mut fx, &third, 80));
        assert_eq!(end, plain);
        let mut fx = Transitions::new(false, 160).with_theme(testdata::pierre());
        draw_view(&mut terminal, &mut fx, &with_message("merged #101"), 0);
        let off = status_text(&draw_view(&mut terminal, &mut fx, &third, 0));
        assert!(!fx.active());
        assert_eq!(off, plain, "nothing with animations off");
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
