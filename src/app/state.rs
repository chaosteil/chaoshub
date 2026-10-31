//! The app state: data and UI state.
//!
//! The split lets `update` borrow `&state.data` and `&mut state.ui` at the
//! same time, for example to give a tab read access to the data while it
//! changes its own state.
#![allow(
    dead_code,
    reason = "update, the tabs and the panes read the fields in later phases"
)]

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};

use crate::app::frame::ModelCache;
use crate::app::models::{CellStyle, Direction, ListModel};
use crate::app::tabs::{TABS, Tab, TabState};
use crate::bus::{Interest, OverlayKind, RateInfo, Scope, Session, Size, SyncError, UiEvent};
use crate::domain::detail::PrDetail;
use crate::domain::keys::{PrKey, RepoKey};
use crate::domain::local::LocalState;
use crate::domain::observed::Observed;
use crate::domain::pr::PullRequest;
use crate::domain::repo::Repo;
use crate::store::Snapshot;

/// The whole state of the app.
#[derive(Debug)]
pub(crate) struct AppState {
    pub data: AppData,
    pub ui: UiState,
    /// The redraw signal. `update` bumps it on every visible change, and
    /// `run` draws when it moved since the last draw.
    pub generation: u64,
    /// The models of the last frame.
    pub cache: ModelCache,
}

/// The data that the workers deliver.
#[derive(Debug)]
pub(crate) struct AppData {
    /// The terminal size.
    pub size: Size,
    pub session: Session,
    pub prs: BTreeMap<PrKey, PullRequest>,
    /// The sync scope of each pull request.
    pub scopes: BTreeMap<PrKey, Scope>,
    /// The jj scan of each repo.
    pub local: BTreeMap<RepoKey, Observed<LocalState>>,
    pub details: BTreeMap<PrKey, Observed<PrDetail>>,
    pub repos: BTreeMap<RepoKey, Repo>,
    /// The newest sync time of the cached pull requests at startup: the
    /// age of the cache. `None` when the cache was empty.
    pub cache_at: Option<DateTime<Utc>>,
    /// The time of the last sync of each scope: the start time of the
    /// sync that delivered the last `PrsSynced`.
    pub synced: HashMap<Scope, DateTime<Utc>>,
    /// The state of the syncs: requested syncs, the last failure and the
    /// rate limit.
    pub sync: SyncInfo,
    /// Counts the changes of the pull requests and the jj scans. The
    /// fuzzy filters score again when it moves.
    pub revision: u64,
    /// The time of the last `Msg::Tick`. Ages are relative to it, so
    /// the models stay pure functions of the state.
    pub now: DateTime<Utc>,
}

/// The state of the UI.
#[derive(Debug)]
pub(crate) struct UiState {
    /// The index of the active tab in the tab registry.
    pub active: usize,
    /// One state per tab, in registry order.
    pub tabs: Vec<TabState>,
    /// The overlay stack, bottom first.
    pub overlays: Vec<Overlay>,
    /// The last status message, until it expires.
    pub status: Option<StatusMsg>,
    /// The events for the transitions since the last draw.
    pub ui_events: Vec<UiEvent>,
    /// The list of the previous tab and the slide direction, until the tab
    /// slide is done.
    pub prev_list: Option<(ListModel, Direction)>,
    /// The interest that was last sent to the workers.
    pub interest: Option<Interest>,
    /// True while the user types the filter of the active tab.
    pub filter_edit: bool,
}

/// The state of the syncs, for the status bar and the terminal status.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct SyncInfo {
    /// The scopes of the syncs that `update` requested and that have no
    /// result yet.
    pub pending: HashSet<Scope>,
    /// The last failure. A key press or the next successful sync clears
    /// it.
    pub failure: Option<Failure>,
    /// The rate limit after the last request.
    pub rate: Option<RateInfo>,
}

/// A sync failure or a worker restart.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Failure {
    /// The failed worker: `sync`, `ci`, `detail`, `prefetch`, or the name
    /// of a restarted worker.
    pub source: &'static str,
    /// The text for the user, for example `offline`.
    pub reason: String,
    /// `Some` for a rate limit: the time when the limit resets.
    pub rate_limited_until: Option<DateTime<Utc>>,
    /// When the failure arrived.
    pub at: DateTime<Utc>,
}

/// A status bar message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StatusMsg {
    pub text: String,
    pub style: CellStyle,
    /// The message leaves the status bar on the first tick after this
    /// time.
    pub expires: DateTime<Utc>,
}

/// The `ui_state` key kind of the filter text of a tab.
pub(crate) const FILTER_KEY: &str = "filter";
/// The `ui_state` key kind of the author filter of a tab.
pub(crate) const AUTHOR_KEY: &str = "author";

/// The `ui_state` key of a setting of a tab, for example `filter.team`.
pub(crate) fn ui_key(kind: &str, tab: &str) -> String {
    format!("{kind}.{tab}")
}

/// The text of a sync error for the user.
pub(crate) fn error_text(err: &SyncError) -> String {
    match err {
        SyncError::Auth => "GitHub login failed, run gh auth login".to_owned(),
        SyncError::RateLimited { until } => {
            format!(
                "rate limited until {}",
                until.with_timezone(&chrono::Local).format("%H:%M")
            )
        }
        SyncError::Offline => "offline".to_owned(),
        SyncError::Other(text) => text.clone(),
    }
}

/// One overlay of the overlay stack.
#[derive(Debug)]
pub(crate) struct Overlay {
    pub kind: OverlayKind,
    /// True while the overlay slides out. `Msg::TransitionsDone` removes
    /// it.
    pub closing: bool,
    /// The own state of the overlay.
    pub state: OverlayState,
}

/// The own state of each overlay kind.
#[derive(Debug)]
pub(crate) enum OverlayState {
    /// The author picker of a tab.
    Authors(AuthorPicker),
}

/// The author picker: the first item shows every author.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AuthorPicker {
    /// The registry index of the tab that the picker filters.
    pub tab: usize,
    /// The authors, sorted. The picker shows `all authors` before them.
    pub authors: Vec<String>,
    /// The index in the picker items: 0 is `all authors`.
    pub cursor: usize,
}

impl AuthorPicker {
    /// The text of the first item.
    pub(crate) const ALL: &'static str = "all authors";

    /// The items of the picker: `all authors`, then the authors.
    pub(crate) fn items(&self) -> Vec<String> {
        std::iter::once(Self::ALL.to_owned())
            .chain(self.authors.iter().cloned())
            .collect()
    }

    /// The author of the cursor item. `None` for `all authors`.
    pub(crate) fn chosen(&self) -> Option<String> {
        self.cursor
            .checked_sub(1)
            .and_then(|i| self.authors.get(i).cloned())
    }
}

impl AppState {
    /// Creates an empty state for a terminal size, with one tab state
    /// per tab of `TABS`.
    pub(crate) fn new(size: Size) -> Self {
        Self::with_tabs(size, &TABS)
    }

    /// Creates an empty state with one tab state per tab of `tabs`.
    pub(crate) fn with_tabs(size: Size, tabs: &[Box<dyn Tab>]) -> Self {
        Self {
            data: AppData {
                size,
                session: Session {
                    login: None,
                    repos: Vec::new(),
                },
                prs: BTreeMap::new(),
                scopes: BTreeMap::new(),
                local: BTreeMap::new(),
                details: BTreeMap::new(),
                repos: BTreeMap::new(),
                cache_at: None,
                synced: HashMap::new(),
                sync: SyncInfo::default(),
                revision: 0,
                now: DateTime::UNIX_EPOCH,
            },
            ui: UiState {
                active: 0,
                tabs: tabs.iter().map(|tab| tab.init_state()).collect(),
                overlays: Vec::new(),
                status: None,
                ui_events: Vec::new(),
                prev_list: None,
                interest: None,
                filter_edit: false,
            },
            generation: 0,
            cache: ModelCache::default(),
        }
    }

    /// Marks a visible change.
    pub(crate) fn bump(&mut self) {
        self.generation += 1;
    }

    /// Shows a status message for `ttl` from now.
    pub(crate) fn message(
        &mut self,
        text: impl Into<String>,
        style: CellStyle,
        ttl: chrono::TimeDelta,
    ) {
        self.ui.status = Some(StatusMsg {
            text: text.into(),
            style,
            expires: self.data.now + ttl,
        });
        self.bump();
    }

    /// Loads the startup snapshot of the cache and the repos of the
    /// session. The cached merge settings fill the settings of the
    /// session repos; a cached repo outside the session keeps no path.
    pub(crate) fn load(&mut self, snapshot: Snapshot, repos: Vec<Repo>) {
        let data = &mut self.data;
        for repo in &repos {
            data.repos.insert(repo.key.clone(), repo.clone());
        }
        data.session.repos = repos;
        for (key, settings) in snapshot.repos {
            data.repos
                .entry(key.clone())
                .or_insert_with(|| Repo {
                    key,
                    path: None,
                    settings: None,
                })
                .settings = Some(settings);
        }
        for (scope, pr) in snapshot.prs {
            data.scopes.insert(pr.key.clone(), scope);
            data.prs.insert(pr.key.clone(), pr);
        }
        data.local.extend(snapshot.local);
        data.cache_at = snapshot.synced_at;
        for (tab, ts) in TABS.iter().zip(self.ui.tabs.iter_mut()) {
            let saved = |kind: &str| {
                snapshot
                    .ui_state
                    .get(&ui_key(kind, tab.id()))
                    .filter(|value| !value.is_empty())
                    .cloned()
            };
            ts.filter = saved(FILTER_KEY).unwrap_or_default();
            ts.author = saved(AUTHOR_KEY);
        }
        self.bump();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tabs::fake::FakeTab;

    #[test]
    fn new_state_is_empty_and_bump_moves_the_generation() {
        let size = Size {
            width: 100,
            height: 30,
        };
        let mut state = AppState::new(size);
        assert_eq!(state.data.size, size);
        assert!(state.data.prs.is_empty());
        assert!(state.ui.overlays.is_empty());
        assert_eq!(state.generation, 0);
        state.bump();
        assert_eq!(state.generation, 1);
    }

    #[test]
    fn load_fills_the_data_from_the_snapshot() {
        use crate::app::testdata::repo_key as key;
        use crate::domain::repo::RepoSettings;
        use crate::domain::test_time;

        let settings = RepoSettings {
            squash: true,
            merge: false,
            rebase: false,
            default_branch: None,
        };
        let pr = crate::app::testdata::pr("r", 1, "One");
        let snapshot = Snapshot {
            prs: vec![(Scope::Team, pr.clone())],
            repos: vec![(key("r"), settings.clone()), (key("old"), settings.clone())],
            synced_at: Some(test_time()),
            local: Vec::new(),
            ui_state: BTreeMap::new(),
        };
        let session = vec![Repo {
            key: key("r"),
            path: Some("/code/r".into()),
            settings: None,
        }];
        let mut state = AppState::new(Size {
            width: 100,
            height: 30,
        });
        state.load(snapshot, session.clone());
        let data = &state.data;
        assert_eq!(data.session.repos, session);
        assert_eq!(data.prs.get(&pr.key), Some(&pr));
        assert_eq!(data.scopes.get(&pr.key), Some(&Scope::Team));
        assert_eq!(data.cache_at, Some(test_time()));
        assert_eq!(
            data.repos.get(&key("r")),
            Some(&Repo {
                key: key("r"),
                path: Some("/code/r".into()),
                settings: Some(settings.clone()),
            })
        );
        assert_eq!(
            data.repos.get(&key("old")).and_then(|r| r.path.clone()),
            None
        );
        assert_eq!(state.generation, 1);
    }

    #[test]
    fn load_restores_the_filters_of_the_tabs() {
        let ui_state = [
            ("filter.mine", "pane"),
            ("author.team", "bob"),
            ("filter.team", ""),
            ("filter.gone", "old tab"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let snapshot = Snapshot {
            ui_state,
            ..Snapshot::default()
        };
        let mut state = AppState::new(Size {
            width: 100,
            height: 30,
        });
        state.load(snapshot, Vec::new());
        assert_eq!(state.ui.tabs[0].filter, "pane");
        assert_eq!(state.ui.tabs[0].author, None);
        assert_eq!(state.ui.tabs[1].filter, "");
        assert_eq!(state.ui.tabs[1].author.as_deref(), Some("bob"));
    }

    #[test]
    fn with_tabs_makes_one_state_per_tab() {
        let tabs: Vec<Box<dyn Tab>> =
            vec![Box::new(FakeTab::new("a")), Box::new(FakeTab::new("b"))];
        let state = AppState::with_tabs(
            Size {
                width: 100,
                height: 30,
            },
            &tabs,
        );
        assert_eq!(state.ui.tabs.len(), 2);
        assert_eq!(state.ui.active, 0);
    }
}
