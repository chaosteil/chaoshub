//! The `Pane` trait and the pane registry.
//!
//! A pane is one sub-view of the pull request popup. The popup shows the
//! panes whose `applies` is true, in registry order. A new pane is one
//! module plus one line in `PANES`.
#![allow(dead_code, reason = "the popup of a later phase uses the panes")]

mod checks;
mod conversation;
mod stack;

use std::any::Any;
use std::sync::LazyLock;

use crate::app::models::PaneModel;
use crate::app::tabs::{Handled, KeyCx};
use crate::bus::{KeyChord, Size, Target};

/// The state of one pane, made by `Pane::init_state`.
#[derive(Debug)]
pub(crate) struct PaneState {
    pub cursor: usize,
    pub scroll: usize,
    /// The own state of the pane, for example the expanded threads.
    pub ext: Box<dyn Any + Send + Sync>,
}

impl Default for PaneState {
    /// The cursor and the scroll at the top, no own state.
    fn default() -> Self {
        Self {
            cursor: 0,
            scroll: 0,
            ext: Box::new(()),
        }
    }
}

/// One sub-view of the pull request popup.
pub(crate) trait Pane: Send + Sync {
    /// The id, for example `checks`.
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;
    /// The key that selects the pane in the popup, for example `1`.
    fn key(&self) -> char;
    fn applies(&self, target: &Target) -> bool;
    fn init_state(&self) -> PaneState;
    fn model(&self, cx: &KeyCx<'_>, ps: &PaneState, size: Size) -> PaneModel;
    /// `None` when the pane does not handle the key.
    fn handle_key(&self, key: KeyChord, cx: &KeyCx<'_>, ps: &mut PaneState) -> Option<Handled>;
}

/// The pane registry, in popup order.
pub(crate) static PANES: LazyLock<Vec<Box<dyn Pane>>> = LazyLock::new(Vec::new);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tabs::duplicates;

    #[test]
    fn pane_ids_and_keys_are_unique() {
        assert_eq!(duplicates(PANES.iter().map(|pane| pane.id())), [""; 0]);
        assert_eq!(duplicates(PANES.iter().map(|pane| pane.key())), [' '; 0]);
    }
}
