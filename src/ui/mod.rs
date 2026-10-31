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
pub(crate) mod transition;

use ratatui::Frame;

use crate::app::layout::Areas;
use crate::app::models::FrameModel;
use crate::theme::Theme;

/// Draws one frame: the tab bar, the list, the status bar and the overlay
/// stack. It draws nothing yet.
pub(crate) fn view(frame: &mut Frame<'_>, model: &FrameModel, areas: &Areas, theme: &Theme) {
    let _ = (frame, model, areas, theme);
}
