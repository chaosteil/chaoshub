//! Derives the terminal status from the app state.
//!
//! `term_status` is pure. It is part of the `FrameModel`, so it is rebuilt
//! only with the generation. For now every state maps to idle; the rows of
//! the status table (syncs, merges, prompts, failures) come with the
//! features that produce them.

use crate::app::state::AppState;
use crate::bus::{Progress, TermState, TermStatus};

/// The terminal status for the app state.
pub(crate) fn term_status(_state: &AppState) -> TermStatus {
    TermStatus {
        state: TermState::Idle,
        msg: None,
        progress: Progress::Off,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Size;

    #[test]
    fn a_fresh_state_is_idle() {
        let state = AppState::new(Size {
            width: 100,
            height: 30,
        });
        assert_eq!(
            term_status(&state),
            TermStatus {
                state: TermState::Idle,
                msg: None,
                progress: Progress::Off,
            }
        );
    }
}
