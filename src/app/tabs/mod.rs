//! The `Tab` trait and the tab registry.
//!
//! A tab turns the app data into a `ListModel`. A new tab is one module
//! plus one line in `TABS`. The order of `TABS` gives the tab bar, the
//! keys `1` to `9` and the slide direction.
#![allow(
    dead_code,
    reason = "the filter, the target and the help are read in later phases"
)]

pub(crate) mod cells;
mod mine;
mod team;

use std::any::Any;
use std::sync::LazyLock;

use crate::app::layout::Areas;
use crate::app::models::{ListModel, RowItem};
use crate::app::state::AppData;
use crate::bus::{Cmd, Interest, KeyChord, PrRef, Scope, Size, Target, UiEvent};
use crate::domain::pr::PullRequest;

/// The state of one tab. `update` owns one per tab, made by
/// `Tab::init_state`.
#[derive(Debug)]
pub(crate) struct TabState {
    pub cursor: usize,
    pub scroll: usize,
    /// The text of the fuzzy filter.
    pub filter: String,
    /// The own state of the tab, for example its toggles.
    pub ext: Box<dyn Any + Send + Sync>,
}

impl Default for TabState {
    /// The cursor and the scroll at the top, no filter, no own state.
    fn default() -> Self {
        Self {
            cursor: 0,
            scroll: 0,
            filter: String::new(),
            ext: Box::new(()),
        }
    }
}

/// What a tab or a pane can read: the data, never the whole app state.
#[derive(Debug, Clone, Copy)]
pub(crate) struct KeyCx<'a> {
    pub data: &'a AppData,
    /// The target of the selected row, if any.
    pub target: Option<&'a Target>,
    pub areas: &'a Areas,
}

/// The result of a key that a tab or a pane handled.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Handled {
    pub cmds: Vec<Cmd>,
    pub events: Vec<UiEvent>,
    /// True when the key changed something visible.
    pub changed: bool,
}

/// One tab of the main screen.
pub(crate) trait Tab: Send + Sync {
    /// The id, for example `mine`.
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;
    /// The sync scope that feeds the tab, if any.
    fn scope(&self) -> Option<Scope>;
    /// True when the tab shows jj data.
    fn wants_local(&self) -> bool;
    /// The initial state. `ext` holds the tab's own toggles and its
    /// matcher.
    fn init_state(&self) -> TabState;
    fn model(&self, cx: &KeyCx<'_>, ts: &TabState, size: Size) -> ListModel;
    fn interest(&self, cx: &KeyCx<'_>, ts: &TabState) -> Interest;
    /// `None` when the tab does not handle the key.
    fn handle_key(&self, key: KeyChord, cx: &KeyCx<'_>, ts: &mut TabState) -> Option<Handled>;
    fn help(&self) -> Vec<(KeyChord, &'static str)>;
}

/// The fields of a pull request that the workers need.
pub(crate) fn pr_ref(pr: &PullRequest) -> PrRef {
    PrRef {
        key: pr.key.clone(),
        node_id: pr.node_id.clone(),
        head_oid: pr.head.value.head_oid.clone(),
        base_ref: pr.head.value.base_ref.clone(),
    }
}

/// The interest of a tab from its list model: the pull request of the
/// cursor row and the pull requests of the visible rows.
pub(crate) fn list_interest(tab: &dyn Tab, model: &ListModel, height: u16) -> Interest {
    let pr = |item: &RowItem| match item {
        RowItem::Pr(pr) => Some(pr.clone()),
        _ => None,
    };
    Interest {
        tab: tab.id(),
        scope: tab.scope(),
        wants_local: tab.wants_local(),
        selected: model.rows.get(model.cursor).and_then(|row| pr(&row.item)),
        visible: model
            .rows
            .iter()
            .skip(model.scroll)
            .take(usize::from(height))
            .filter_map(|row| pr(&row.item))
            .collect(),
    }
}

/// The tab registry, in tab bar order.
pub(crate) static TABS: LazyLock<Vec<Box<dyn Tab>>> = LazyLock::new(|| vec![Box::new(mine::Mine)]);

/// The values that occur more than once, sorted.
#[cfg(test)]
pub(crate) fn duplicates<T: Ord + Clone>(values: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut sorted: Vec<T> = values.into_iter().collect();
    sorted.sort();
    let mut dups: Vec<T> = sorted
        .windows(2)
        .filter(|pair| pair[0] == pair[1])
        .map(|pair| pair[0].clone())
        .collect();
    dups.dedup();
    dups
}

/// A tab for the tests of the frame builder and of `update`.
#[cfg(test)]
pub(crate) mod fake {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// Counts its model builds and returns `handled` for every key.
    pub(crate) struct FakeTab {
        pub id: &'static str,
        /// The number of model builds, shared with the test.
        pub models: Arc<AtomicUsize>,
        pub handled: Option<Handled>,
    }

    impl FakeTab {
        pub(crate) fn new(id: &'static str) -> Self {
            Self {
                id,
                models: Arc::new(AtomicUsize::new(0)),
                handled: None,
            }
        }

        /// A counter of the model builds that stays readable after the
        /// tab moves into a registry.
        pub(crate) fn counter(&self) -> Arc<AtomicUsize> {
            Arc::clone(&self.models)
        }
    }

    impl Tab for FakeTab {
        fn id(&self) -> &'static str {
            self.id
        }

        fn title(&self) -> &'static str {
            self.id
        }

        fn scope(&self) -> Option<Scope> {
            None
        }

        fn wants_local(&self) -> bool {
            false
        }

        fn init_state(&self) -> TabState {
            TabState::default()
        }

        fn model(&self, _cx: &KeyCx<'_>, ts: &TabState, _size: Size) -> ListModel {
            self.models.fetch_add(1, Ordering::SeqCst);
            ListModel {
                columns: Vec::new(),
                rows: Vec::new(),
                cursor: ts.cursor,
                scroll: ts.scroll,
            }
        }

        fn interest(&self, _cx: &KeyCx<'_>, ts: &TabState) -> Interest {
            Interest {
                tab: self.id,
                scope: None,
                wants_local: ts.cursor > 0,
                selected: None,
                visible: Vec::new(),
            }
        }

        fn handle_key(
            &self,
            _key: KeyChord,
            _cx: &KeyCx<'_>,
            ts: &mut TabState,
        ) -> Option<Handled> {
            let handled = self.handled.clone()?;
            ts.cursor += 1;
            Some(handled)
        }

        fn help(&self) -> Vec<(KeyChord, &'static str)> {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicates_finds_repeated_values() {
        assert_eq!(duplicates(["a", "b", "a", "c", "a"]), ["a"]);
        assert_eq!(duplicates(["a", "b"]), [""; 0]);
    }

    #[test]
    fn tab_ids_and_titles_are_unique() {
        assert_eq!(duplicates(TABS.iter().map(|tab| tab.id())), [""; 0]);
        assert_eq!(duplicates(TABS.iter().map(|tab| tab.title())), [""; 0]);
        assert!(TABS.len() <= 9, "the keys 1 to 9 select a tab");
    }
}
