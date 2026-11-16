//! The app: state, `update`, the command runtime, tabs, panes, models and
//! layout. No ratatui.

mod age;
pub(crate) mod exec;
mod filter;
mod frame;
mod help;
pub(crate) mod keymap;
pub(crate) mod layout;
pub(crate) mod measure;
mod merge;
pub(crate) mod models;
pub(crate) mod nav;
mod notify;
mod panes;
mod state;
mod tabs;
mod target;
mod term;
#[cfg(test)]
pub(crate) mod testdata;
mod update;

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

pub(crate) use crate::app::frame::SPINNER_FRAME;
use crate::app::keymap::Keymap;
use crate::app::layout::Areas;
use crate::app::models::FrameModel;
pub(crate) use crate::app::notify::{Notice, NotifySettings};
use crate::app::state::AppState;
pub(crate) use crate::app::state::DetailPane;
use crate::bus::{Cmd, Interest, Msg, Size, UiEvent};
use crate::config::schema::FiltersConfig;
use crate::domain::keys::PrKey;
use crate::domain::repo::Repo;
use crate::store::Snapshot;

/// The app: the state and the message handler. `run` drives it.
#[derive(Debug)]
pub(crate) struct App {
    state: AppState,
    keymap: Keymap,
}

impl App {
    /// Creates the app for a terminal size and a key map, with the
    /// startup snapshot of the cache and the repos of the session. `now`
    /// is the start time; the ages of the first frame count from it.
    /// `key_hints` is the `[theme] key_hints` switch.
    pub(crate) fn new(
        size: Size,
        keymap: Keymap,
        key_hints: bool,
        snapshot: Snapshot,
        repos: Vec<Repo>,
        now: DateTime<Utc>,
    ) -> Self {
        let mut state = AppState::new(size);
        state.cache.set_key_hints(keymap.clone(), key_hints);
        state.data.now = now;
        state.load(snapshot, repos);
        update::start(&mut state);
        Self { state, keymap }
    }

    /// Sets the list filters and updates the first selection and worker
    /// interest before the first frame.
    pub(crate) fn with_filters(mut self, filters: FiltersConfig) -> Self {
        if self.state.data.filters != filters {
            self.state.data.filters = filters;
            self.state.data.revision += 1;
            self.state.bump();
            update::start(&mut self.state);
        }
        self
    }

    /// Sets the `[actions.<id>]` tables of the configuration. Each
    /// selection carries the table of its action.
    pub(crate) fn with_action_config(mut self, tables: BTreeMap<String, toml::Table>) -> Self {
        self.state.data.action_config = tables;
        self
    }

    /// True when the list shows the header row.
    #[cfg(test)]
    pub(crate) fn column_headers(&self) -> bool {
        self.state.ui.column_headers
    }

    /// Sets the `[theme] column_headers` switch. A value in the cache wins
    /// over it.
    pub(crate) fn with_column_headers(mut self, on: bool) -> Self {
        let ui = &mut self.state.ui;
        ui.column_headers = ui.saved_column_headers.unwrap_or(on);
        self.state.bump();
        self
    }

    /// Sets the command of the detail pane. With `Some`, `enter` opens the
    /// pull request in a pane. With `None`, it opens the sidebar.
    pub(crate) fn with_detail_pane(mut self, detail: Option<DetailPane>) -> Self {
        self.state.data.detail = detail;
        self
    }

    /// Sets the cells that the theme moves the text of a selected row to
    /// the right. The panes measure their tree rows with it.
    pub(crate) fn with_selection_shift(mut self, shift: u16) -> Self {
        self.state.data.selection_shift = shift;
        self.state.bump();
        self
    }

    /// Sets the `[theme] animations` switch. With `false`, the merge badge
    /// does not pulse.
    pub(crate) fn with_animations(mut self, on: bool) -> Self {
        self.state.data.animations = on;
        self.state.bump();
        self
    }

    /// Turns the notification of new pull requests on with `Some`.
    pub(crate) fn with_notify(mut self, settings: Option<NotifySettings>) -> Self {
        self.state.data.notify.settings = settings;
        update::start(&mut self.state);
        self
    }

    /// The notice of new pull requests that the terminal shows, until
    /// the next key press.
    pub(crate) fn notice(&self) -> Option<&Notice> {
        self.state.data.notify.notice.as_ref()
    }

    /// Pins the pull request `key`: the pinned mode of `chaoshub pr`. The
    /// popup of `key` is in place from the first frame, without a slide,
    /// and the interest is the one of the pinned pull request.
    pub(crate) fn pinned(mut self, key: PrKey) -> Self {
        update::pin(&mut self.state, key);
        self
    }

    /// Applies one message and returns the commands to execute.
    pub(crate) fn update(&mut self, msg: Msg) -> Vec<Cmd> {
        update::update(&mut self.state, &self.keymap, msg)
    }

    /// The model of the next frame, rebuilt only when the generation
    /// moved.
    pub(crate) fn frame(&mut self) -> &FrameModel {
        frame::frame_model(&mut self.state)
    }

    /// The interest that `update` computed last, for the first value of
    /// the interest watch.
    pub(crate) fn interest(&self) -> Option<&Interest> {
        self.state.ui.interest.as_ref()
    }

    /// True while a requested sync, a prefetch batch or a merge runs (see
    /// `AppData::busy`): the status bar shows a spinner, and `run` ticks
    /// every `SPINNER_FRAME`.
    pub(crate) fn busy(&self) -> bool {
        self.state.data.busy()
    }

    /// The redraw signal.
    pub(crate) fn generation(&self) -> u64 {
        self.state.generation
    }

    /// The terminal size.
    #[cfg(test)]
    pub(crate) fn size(&self) -> Size {
        self.state.data.size
    }

    /// The areas of the screen, with the key hint bar when the key hints
    /// are on.
    pub(crate) fn areas(&self) -> Areas {
        frame::areas(&self.state)
    }

    /// True when the frame still holds what a transition shows: an
    /// overlay that slides out. Without a running transition, `run` then
    /// sends `Msg::TransitionsDone`.
    pub(crate) fn transition_leftovers(&self) -> bool {
        self.state.ui.overlays.iter().any(|o| o.closing)
    }

    /// Takes the UI events for the transitions.
    pub(crate) fn take_ui_events(&mut self) -> Vec<UiEvent> {
        std::mem::take(&mut self.state.ui.ui_events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::models::RowItem;
    use crate::app::testdata;
    use crate::bus::Scope;
    use crate::domain::test_time;

    fn numbers(app: &mut App) -> Vec<u64> {
        app.frame()
            .list
            .rows
            .iter()
            .filter_map(|row| match &row.item {
                RowItem::Pr(pr) => Some(pr.key.number),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_cutoff_filters_cached_and_live_rows_but_keeps_direct_pr_access() {
        let mut old = testdata::pr("chaoshub", 1, "Old");
        old.head.value.created_at = Some("2026-10-29T23:59:59Z".parse().unwrap());
        let mut included = testdata::pr("chaoshub", 2, "Included");
        included.head.value.created_at = Some("2026-10-30T00:00:00Z".parse().unwrap());
        let mut unknown = testdata::pr("chaoshub", 3, "Old cache");
        unknown.head.value.created_at = None;
        let snapshot = Snapshot {
            prs: vec![
                (Scope::Mine, old.clone()),
                (Scope::Mine, included.clone()),
                (Scope::Mine, unknown),
            ],
            ..Snapshot::default()
        };
        let mut app = App::new(
            Size {
                width: 100,
                height: 30,
            },
            Keymap::default(),
            true,
            snapshot,
            Vec::new(),
            "2026-11-13T00:00:00Z".parse().unwrap(),
        );
        assert_eq!(numbers(&mut app), [1, 2, 3]);
        app = app.with_filters(FiltersConfig {
            max_age: Some(chrono::TimeDelta::weeks(2)),
        });
        assert_eq!(numbers(&mut app), [2]);
        assert_eq!(
            app.interest()
                .unwrap()
                .selected
                .as_ref()
                .unwrap()
                .key
                .number,
            2
        );
        let mut recent = included;
        recent.key.number = 4;
        let mut excluded = old.clone();
        excluded.key.number = 5;
        app.update(Msg::PrsSynced {
            scope: Scope::Mine,
            prs: vec![recent, excluded],
            seen: None,
            started_at: test_time(),
            last: true,
        });
        assert_eq!(numbers(&mut app), [2, 4]);
        old.head.value.updated_at += chrono::TimeDelta::days(1);
        old.head.observed_at += chrono::TimeDelta::days(1);
        app.update(Msg::PrRefreshed(Box::new(old.clone())));
        assert_eq!(numbers(&mut app), [2, 4]);
        assert!(app.state.data.prs.contains_key(&old.key));
        app = app.pinned(old.key.clone());
        assert_eq!(
            app.interest().unwrap().selected.as_ref().unwrap().key,
            old.key
        );
    }
    #[test]
    fn the_age_limit_uses_elapsed_time_and_includes_the_boundary() {
        let config =
            crate::config::Config::parse("[filters]\nmax_age = \"2 weeks\"", None).unwrap();
        let now = "2026-11-13T12:34:56Z".parse().unwrap();
        let mut pr = testdata::pr("chaoshub", 1, "Old pull request");
        pr.head.value.updated_at = now;
        for (created, included) in [
            (Some("2026-10-30T12:34:55.999999999Z"), false),
            (Some("2026-10-30T12:34:56Z"), true),
            (Some("2026-10-30T05:34:56-07:00"), true),
            (Some("2026-10-30T23:59:59Z"), true),
            (Some("2026-11-01T00:00:00Z"), true),
            (None, false),
        ] {
            pr.head.value.created_at = created.map(|value| value.parse().unwrap());
            assert_eq!(config.filters.matches(&pr, now), included, "{created:?}");
            assert!(FiltersConfig::default().matches(&pr, now));
        }
    }
}
