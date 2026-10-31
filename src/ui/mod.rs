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
/// stack. It draws nothing yet.
pub(crate) fn view(frame: &mut Frame<'_>, model: &FrameModel, areas: &Areas, theme: &Theme) {
    let _ = (frame, model, areas, theme);
}
