//! The `Pane` trait and the pane registry.
//!
//! A pane is one sub-view of the pull request popup. The popup shows the
//! panes whose `applies` is true, in registry order. A new pane is one
//! module plus one line in `PANES`.

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
    #[allow(
        dead_code,
        reason = "the conversation pane keeps its expanded threads here"
    )]
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
    #[allow(
        dead_code,
        reason = "the registry test reads it, and phase E keys pane actions by it"
    )]
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
pub(crate) static PANES: LazyLock<Vec<Box<dyn Pane>>> =
    LazyLock::new(|| vec![Box::new(checks::Checks)]);

/// The registry indexes of the panes that apply to `target`.
pub(crate) fn applicable(panes: &[Box<dyn Pane>], target: &Target) -> Vec<usize> {
    panes
        .iter()
        .enumerate()
        .filter(|(_, pane)| pane.applies(target))
        .map(|(i, _)| i)
        .collect()
}

/// The number of items that the cursor of a pane moves over: rows,
/// nodes or lines.
pub(crate) fn items(model: &PaneModel) -> usize {
    match model {
        PaneModel::Table { rows, .. } => rows.len(),
        PaneModel::Lines { lines, .. } => lines.len(),
        PaneModel::Tree { nodes } => nodes.len(),
    }
}

/// A pane for the tests of the popup.
#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use crate::app::models::{Cell, CellStyle};
    use crate::bus::{Cmd, KeyCode};

    /// Shows `lines` numbered lines. `x` returns `Cmd::OpenUrl` with the
    /// cursor line; it applies only when `only` is `None` or the number of
    /// the pull request.
    pub(crate) struct FakePane {
        pub id: &'static str,
        pub key: char,
        pub lines: usize,
        pub only: Option<u64>,
    }

    impl Pane for FakePane {
        fn id(&self) -> &'static str {
            self.id
        }

        fn title(&self) -> &'static str {
            self.id
        }

        fn key(&self) -> char {
            self.key
        }

        fn applies(&self, target: &Target) -> bool {
            match (target, self.only) {
                (_, None) => true,
                (Target::Pr { pr, .. }, Some(number)) => pr.key.number == number,
                (Target::Local { .. }, Some(_)) => false,
            }
        }

        fn init_state(&self) -> PaneState {
            PaneState::default()
        }

        fn model(&self, _cx: &KeyCx<'_>, _ps: &PaneState, _size: Size) -> PaneModel {
            PaneModel::Tree {
                nodes: (0..self.lines)
                    .map(|i| crate::app::models::TreeNode {
                        depth: 0,
                        expanded: None,
                        cells: vec![Cell {
                            text: format!("{} {i}", self.id),
                            style: CellStyle::Normal,
                        }],
                    })
                    .collect(),
            }
        }

        fn handle_key(
            &self,
            key: KeyChord,
            _cx: &KeyCx<'_>,
            ps: &mut PaneState,
        ) -> Option<Handled> {
            (key.code == KeyCode::Char('x')).then(|| Handled {
                cmds: vec![Cmd::OpenUrl(format!("{} {}", self.id, ps.cursor))],
                events: Vec::new(),
                changed: false,
            })
        }
    }
}

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
