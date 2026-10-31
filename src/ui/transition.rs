//! Transitions on tachyonfx. The effects come in a later phase. For now
//! no transition runs.

use std::time::Duration;

use ratatui::Frame;

use crate::app::layout::Areas;
use crate::app::models::FrameModel;
use crate::bus::UiEvent;

/// Turns UI events into effects and runs them over the frame buffer.
#[derive(Debug)]
pub(crate) struct Transitions {
    /// False when the configuration turns animations off.
    enabled: bool,
    /// The duration of one transition.
    duration: Duration,
}

#[allow(
    clippy::unused_self,
    reason = "the effects of a later phase use self in each method"
)]
impl Transitions {
    /// Creates the transitions. `enabled` comes from `[theme] animations`
    /// and `duration_ms` from `[theme] animation_ms`.
    pub(crate) fn new(enabled: bool, duration_ms: u64) -> Self {
        Self {
            enabled,
            duration: Duration::from_millis(duration_ms),
        }
    }

    /// Starts one effect per event. Does nothing yet.
    pub(crate) fn apply(&mut self, events: Vec<UiEvent>) {
        drop(events);
    }

    /// Draws the previous list into the scratch buffer while a tab slide
    /// runs. Does nothing yet.
    pub(crate) fn prepare(&mut self, model: &FrameModel, areas: &Areas) {
        let _ = (model, areas);
    }

    /// Runs the effects over the frame buffer. Does nothing yet.
    pub(crate) fn process(&mut self, frame: &mut Frame<'_>, elapsed: Duration) {
        let _ = (frame, elapsed);
    }

    /// True while a transition runs. For now it is always false.
    pub(crate) fn active(&self) -> bool {
        false
    }
}
