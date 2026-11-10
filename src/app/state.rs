//! The app state: data and UI state.
//!
//! The split lets `update` borrow `&state.data` and `&mut state.ui` at the
//! same time, for example to give a tab read access to the data while it
//! changes its own state.
#![allow(
    dead_code,
    reason = "update, the tabs and the panes read the fields in later phases"
)]

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;

use chrono::{DateTime, Utc};

use crate::actions::{Action, Applicability};
use crate::app::frame::{self, ModelCache};
use crate::app::keymap::Keymap;
use crate::app::models::{CellStyle, HelpModel};
use crate::app::panes::PaneState;
use crate::app::tabs::{TABS, Tab, TabState};
use crate::bus::{
    ChooseSpec, ConfirmSpec, Interest, OverlayKind, PromptSpec, RateInfo, ReviewKind, Scope,
    Selection, Session, Size, SyncError, Target, TargetKey, UiEvent,
};
use crate::domain::agent::{AgentResults, ToolKind};
use crate::domain::codeview::Comments;
use crate::domain::detail::PrDetail;
use crate::domain::files::PrFiles;
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
    /// The pull requests whose conversation the detail worker reads from
    /// GitHub now.
    pub details_loading: BTreeSet<PrKey>,
    /// The reason of the last failed detail load of each pull request,
    /// until a load succeeds.
    pub detail_errors: BTreeMap<PrKey, String>,
    pub files: BTreeMap<PrKey, Observed<PrFiles>>,
    /// The pull requests whose changed files the detail worker reads
    /// from GitHub now.
    pub files_loading: BTreeSet<PrKey>,
    /// The reason of the last failed files load of each pull request,
    /// until a load succeeds.
    pub files_errors: BTreeMap<PrKey, String>,
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
    /// The `[actions.<id>]` tables of the configuration, by action id.
    pub action_config: BTreeMap<String, toml::Table>,
    /// The panes of the terminal actions, by action id and target. One
    /// pane per key.
    pub panes: HashMap<(&'static str, TargetKey), PaneHandleState>,
    /// The state of the last async merge, from `ActionEvent::MergeStatus`.
    /// A key press clears a final state.
    pub merge: Option<MergeState>,
    /// The prefetch batch that runs, for the terminal progress bar.
    pub prefetch: Option<PrefetchBatch>,
    /// The command of the detail pane. `None` opens the sidebar.
    pub detail: Option<DetailPane>,
    /// The pr-agent results of each pull request, from the result files.
    pub agent: BTreeMap<PrKey, AgentResults>,
    /// The pr-agent runs that run now. One run per tool per pull request.
    pub agent_jobs: BTreeSet<(PrKey, ToolKind)>,
    /// The unsent codeview comments of each pull request, from the
    /// comment files, or the error of a file.
    pub codeview: BTreeMap<PrKey, Result<Comments, String>>,
    /// The cells that the theme moves the text of a selected row to the
    /// right (`Selection::shift`). The tree rows of a pane are measured
    /// without these cells, as `ui` draws them.
    pub selection_shift: u16,
}

/// The command of the detail pane, as plain data from `run`: `app`
/// adds `pr OWNER/NAME NUMBER` to the argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DetailPane {
    /// The binary and the flags of the list process.
    pub argv: Vec<String>,
    /// The directory of the list process, for a repo without a path.
    pub cwd: PathBuf,
    /// The `[detail] split` direction.
    pub split: Option<String>,
    /// The `[detail] ratio` in percent.
    pub ratio: Option<u8>,
}

/// The state of an async merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MergeState {
    /// The merged pull request: the top entry for a stack.
    pub key: PrKey,
    pub progress: crate::bus::MergeProgress,
    /// When the state arrived.
    pub at: DateTime<Utc>,
}

/// The progress of a prefetch batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PrefetchBatch {
    /// The fetched refs.
    pub done: usize,
    /// The refs of the batch.
    pub total: usize,
}

impl AppData {
    /// True while a sync, a prefetch batch, a conversation load, a files
    /// load or a pr-agent run runs. The status bar then shows a spinner.
    pub(crate) fn busy(&self) -> bool {
        !self.sync.pending.is_empty()
            || !self.sync.running.is_empty()
            || self.prefetch.is_some()
            || !self.details_loading.is_empty()
            || !self.files_loading.is_empty()
            || !self.agent_jobs.is_empty()
    }

    /// True while a pane of any action is open for `key`.
    pub(crate) fn pane_open(&self, key: &TargetKey) -> bool {
        self.panes.keys().any(|(_, open)| open == key)
    }
}

/// The state of a pane of a terminal action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaneHandleState {
    /// `Cmd::RunTerminal` went out; the pane did not start yet.
    Starting,
    /// `Msg::PaneStarted` came.
    Running,
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
    /// The interest that was last sent to the workers.
    pub interest: Option<Interest>,
    /// True while the user types the filter of the active tab.
    pub filter_edit: bool,
    /// The pull request of the pinned mode (`chaoshub pr`). Its popup is
    /// the overlay at index 0 and never closes. `None` in the list mode.
    pub pinned: Option<PrKey>,
    /// True when the list shows the header row with the column titles.
    pub column_headers: bool,
    /// The `column_headers` value in the `ui_state` table of the cache,
    /// if any. It wins over `[theme] column_headers`.
    pub saved_column_headers: Option<bool>,
    /// The codeview send that waits for a new read of the comment files:
    /// the pull request and the chosen comment ids.
    pub codeview_send: Option<(PrKey, Vec<String>)>,
}

/// The state of the syncs, for the status bar and the terminal status.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct SyncInfo {
    /// The scopes of the syncs that `update` requested and that have no
    /// result yet.
    pub pending: HashSet<Scope>,
    /// The scopes of the syncs that the sync worker started and did not
    /// end: requested ones and timed ones.
    pub running: HashSet<Scope>,
    /// The last failure. A key press or the next successful sync clears
    /// it.
    pub failure: Option<Failure>,
    /// The rate limit after the last request.
    pub rate: Option<RateInfo>,
    /// The lasting state of the status bar. A key press does not clear
    /// it.
    pub condition: Option<Condition>,
}

/// A lasting state of the status bar: a rate limit, no network, or a
/// worker restart. A message on top of it hides it until the message
/// expires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Condition {
    /// GitHub refuses requests until `until`.
    RateLimited { until: DateTime<Utc> },
    /// A request found no network. The next success clears it.
    Offline,
    /// The supervisor restarts the worker `name` `secs` seconds after
    /// the panic, at `at`.
    Restarting {
        name: &'static str,
        secs: u64,
        at: DateTime<Utc>,
    },
}

impl Condition {
    /// The status bar message: `rate limited until HH:MM` in warn until
    /// the reset time, `offline` in danger until the next success, and
    /// `<name> worker restarting in Ns` in warn until the restart.
    pub(crate) fn message(&self) -> StatusMsg {
        match self {
            Self::RateLimited { until } => StatusMsg {
                text: error_text(&SyncError::RateLimited { until: *until }),
                style: CellStyle::Warn,
                expires: *until,
            },
            Self::Offline => StatusMsg {
                text: error_text(&SyncError::Offline),
                style: CellStyle::Danger,
                expires: DateTime::<Utc>::MAX_UTC,
            },
            Self::Restarting { name, secs, at } => StatusMsg {
                text: format!("{name} worker restarting in {secs}s"),
                style: CellStyle::Warn,
                expires: *at,
            },
        }
    }

    /// True while the state lasts at `now`.
    pub(crate) fn holds(&self, now: DateTime<Utc>) -> bool {
        match self {
            Self::RateLimited { until } => now < *until,
            Self::Offline => true,
            Self::Restarting { at, .. } => now < *at,
        }
    }
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
/// The `ui_state` key of the column header switch: `true` or `false`.
pub(crate) const COLUMN_HEADERS_KEY: &str = "column_headers";

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
    /// The pull request popup.
    Popup(PopupState),
    /// The author picker of a tab.
    Authors(AuthorPicker),
    /// The actions menu of a list row.
    Actions(ActionMenu),
    /// The confirm popup of an action: `y` or Enter runs it.
    Confirm {
        action: ActionDialog,
        spec: ConfirmSpec,
    },
    /// The prompt popup of an action: Enter runs it with the text.
    Prompt {
        action: ActionDialog,
        spec: PromptSpec,
        text: String,
    },
    /// The choice popup of the codeview send: the key of an option picks
    /// the event of the review of the comments `ids`.
    Choose {
        action: ActionDialog,
        spec: ChooseSpec,
        ids: Vec<String>,
    },
    /// The multi-line text of the review of the codeview comments `ids`.
    /// `ctrl-s` runs the action with the text.
    ReviewText {
        action: ActionDialog,
        title: String,
        ids: Vec<String>,
        event: ReviewKind,
        text: String,
        /// The error that the popup shows, for example an empty text of
        /// a change request.
        error: Option<String>,
    },
    /// The help overlay: the key bindings that it shows, and the index
    /// of its first visible line.
    Help { help: HelpModel, scroll: usize },
    /// The output popup of an action: the lines of its
    /// `ActionEvent::Output` events, at most `OUTPUT_LINES`, and the index
    /// of the first visible line. On the last line, the popup follows
    /// the new lines.
    Output {
        id: &'static str,
        key: TargetKey,
        title: String,
        lines: Vec<String>,
        scroll: usize,
    },
}

/// The most lines that an output popup keeps. Older lines go first.
pub(crate) const OUTPUT_LINES: usize = 2000;

/// The action that a confirm or a prompt popup runs, and its selection.
#[derive(Debug)]
pub(crate) struct ActionDialog {
    pub id: &'static str,
    pub selection: Box<Selection>,
}

/// The pull request popup: the pull request, the active pane and one
/// state per pane of the registry.
#[derive(Debug)]
pub(crate) struct PopupState {
    pub key: PrKey,
    /// The registry index of the active pane.
    pub active: usize,
    /// One state per pane, in registry order.
    pub panes: Vec<PaneState>,
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

/// The actions menu of a list row: one item for each action that is not
/// hidden for the row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ActionMenu {
    /// The row that the actions run on.
    pub target: Box<Target>,
    /// The ids of the actions, in display order.
    pub ids: Vec<&'static str>,
    /// The index of the cursor item.
    pub cursor: usize,
}

impl ActionMenu {
    /// The items of the menu: the key and the label of each action of
    /// `actions` in `ids`. A disabled action also shows its reason in
    /// parentheses. An action without a key shows only its label.
    pub(crate) fn items(
        &self,
        data: &AppData,
        keymap: &Keymap,
        actions: &[Box<dyn Action>],
    ) -> Vec<String> {
        let states = frame::applicability(data, actions, (*self.target).clone());
        self.ids
            .iter()
            .map(|&id| {
                let Some((action, state)) = actions
                    .iter()
                    .zip(&states)
                    .find(|(action, _)| action.id() == id)
                else {
                    return id.replace('_', " ");
                };
                let label = action.label();
                let text = match keymap.chords(id).first() {
                    Some(&chord) => format!("{}  {label}", frame::key_text(chord)),
                    None => label.to_owned(),
                };
                match state {
                    Applicability::Disabled(reason) => format!("{text} ({reason})"),
                    Applicability::Enabled | Applicability::Hidden => text,
                }
            })
            .collect()
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
                details_loading: BTreeSet::new(),
                detail_errors: BTreeMap::new(),
                files: BTreeMap::new(),
                files_loading: BTreeSet::new(),
                files_errors: BTreeMap::new(),
                repos: BTreeMap::new(),
                cache_at: None,
                synced: HashMap::new(),
                sync: SyncInfo::default(),
                revision: 0,
                now: DateTime::UNIX_EPOCH,
                action_config: BTreeMap::new(),
                panes: HashMap::new(),
                merge: None,
                prefetch: None,
                detail: None,
                agent: BTreeMap::new(),
                agent_jobs: BTreeSet::new(),
                codeview: BTreeMap::new(),
                selection_shift: 0,
            },
            ui: UiState {
                active: 0,
                tabs: tabs.iter().map(|tab| tab.init_state()).collect(),
                overlays: Vec::new(),
                status: None,
                ui_events: Vec::new(),
                interest: None,
                filter_edit: false,
                pinned: None,
                column_headers: true,
                saved_column_headers: None,
                codeview_send: None,
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
        self.ui.saved_column_headers = match snapshot
            .ui_state
            .get(COLUMN_HEADERS_KEY)
            .map(String::as_str)
        {
            Some("true") => Some(true),
            Some("false") => Some(false),
            _ => None,
        };
        if let Some(on) = self.ui.saved_column_headers {
            self.ui.column_headers = on;
        }
        self.bump();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tabs::fake::FakeTab;

    #[test]
    fn the_column_header_switch_comes_back_from_the_cache() {
        let size = Size {
            width: 100,
            height: 30,
        };
        let load = |value: Option<&str>| {
            let mut state = AppState::new(size);
            let mut snapshot = Snapshot::default();
            if let Some(value) = value {
                snapshot
                    .ui_state
                    .insert(COLUMN_HEADERS_KEY.into(), value.into());
            }
            state.load(snapshot, Vec::new());
            (state.ui.column_headers, state.ui.saved_column_headers)
        };
        assert_eq!(load(None), (true, None));
        assert_eq!(load(Some("false")), (false, Some(false)));
        assert_eq!(load(Some("true")), (true, Some(true)));
        assert_eq!(load(Some("maybe")), (true, None), "a bad value is ignored");
    }

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
            squash_title_default: None,
            squash_message_default: None,
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
        assert_eq!(state.ui.tabs[2].filter, "");
        assert_eq!(state.ui.tabs[2].author.as_deref(), Some("bob"));
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
