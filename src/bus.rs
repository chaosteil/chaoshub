//! Contracts between the layers: `Msg`, `Cmd`, `Interest`, `Session`,
//! `Selection`, specs and errors. Plain data only.
//!
//! `bus` imports only `domain` and the crossterm key types. It re-exports
//! the key types, so `app` reads keys without a crossterm import. No
//! adapter type appears here.
#![allow(
    dead_code,
    reason = "the outer layers use the contracts in later phases"
)]

use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
pub(crate) use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::domain::agent::{AgentResults, Record, Tool, ToolKind};
use crate::domain::check::Check;
use crate::domain::detail::PrDetail;
use crate::domain::files::PrFiles;
use crate::domain::keys::{PrKey, RepoKey};
use crate::domain::local::{LocalMatch, LocalState};
use crate::domain::observed::Observed;
use crate::domain::pr::{CiState, PullRequest};
use crate::domain::repo::Repo;

/// The sync scope: the pull requests of the user, or of colleagues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Scope {
    Mine,
    Team,
}

/// A key with its modifiers, for key bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct KeyChord {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

/// The kind of a mouse event that `app` acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MouseKind {
    /// A press of the left button.
    Click,
    ScrollUp,
    ScrollDown,
}

/// The target that `ui` drew at a cell. `run` finds it from the mouse
/// position, so `app` knows no screen geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Hit {
    /// The tab label at this index in the tab bar.
    Tab(usize),
    /// The list row at this absolute index.
    Row(usize),
    /// The key hint of this chord.
    Key(KeyChord),
    /// The pane tab at this index in the tabs of the popup.
    PaneTab(usize),
    /// The table row or tree node at this absolute index in the active
    /// pane.
    PaneRow(usize),
    /// The picker item at this index.
    Item(usize),
    /// A cell of the top overlay with no other target.
    Inside,
    /// A cell outside of the overlays while an overlay shows.
    Outside,
}

/// A terminal size in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Size {
    pub width: u16,
    pub height: u16,
}

/// The fields of a pull request that workers need without a store read.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PrRef {
    pub key: PrKey,
    pub node_id: String,
    pub head_oid: String,
    pub base_ref: String,
}

/// What the user looks at. Selection-driven workers read only this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Interest {
    /// The id of the active tab.
    pub tab: &'static str,
    /// The sync scope that feeds the tab, if any.
    pub scope: Option<Scope>,
    /// True when the tab shows jj data.
    pub wants_local: bool,
    pub selected: Option<PrRef>,
    pub visible: Vec<PrRef>,
}

/// The login of the user and the set of repos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Session {
    /// `None` until the startup worker reads it.
    pub login: Option<String>,
    pub repos: Vec<Repo>,
}

/// Identifies the target of an action or a pane.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum TargetKey {
    Pr(PrKey),
}

/// The row that an action applies to.
#[derive(Debug, Clone, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "a target is built once per key press and travels boxed inside Cmd"
)]
pub(crate) enum Target {
    Pr {
        pr: PullRequest,
        /// The loaded stack entries, position 1 first.
        stack: Vec<PullRequest>,
        local: LocalMatch,
        repo: Repo,
    },
}

impl Target {
    /// The key of the target.
    pub(crate) fn key(&self) -> TargetKey {
        match self {
            Target::Pr { pr, .. } => TargetKey::Pr(pr.key.clone()),
        }
    }

    /// The repo of the target.
    pub(crate) fn repo(&self) -> &RepoKey {
        match self {
            Target::Pr { repo, .. } => &repo.key,
        }
    }
}

/// The input of an action: the target, the user and the action's own
/// configuration.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Selection {
    pub target: Target,
    /// The login of the user.
    pub me: String,
    /// The `[actions.<id>]` table of the configuration.
    pub action_config: Option<toml::Table>,
}

/// The kinds of overlays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum OverlayKind {
    Popup,
    Confirm,
    Prompt,
    Output,
    Picker,
    Help,
}

/// The reason of a blocked program status (OSC 7501 `kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BlockedKind {
    Permission,
    Question,
    Auth,
}

/// The program status (OSC 7501 `state`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TermState {
    Idle,
    Working,
    Blocked { kind: BlockedKind },
    Done,
    Error,
    Clear,
}

/// The progress bar (OSC 9;4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Progress {
    /// In progress, 0 to 100.
    Percent(u8),
    Indeterminate,
    Error,
    Paused,
    Off,
}

/// The terminal status that `run` writes after a draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TermStatus {
    pub state: TermState,
    pub msg: Option<String>,
    pub progress: Progress,
}

/// A request to the sync worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SyncReq {
    Full(Scope),
    Refresh(Vec<PrRef>),
    /// Reads one pull request by number. It needs no node id, so it also
    /// reads a pull request that is not in the cache.
    Load(PrKey),
}

/// A request to the jj worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum JjReq {
    Scan(RepoKey),
}

/// A command to run in a terminal: in a detached pane or in the
/// foreground.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LaunchRequest {
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub label: String,
    /// True when the pane fills the window.
    pub zoom: bool,
    /// The direction of the new pane. `None` uses the default of the
    /// launcher.
    pub split: Option<String>,
    /// The percent of the split that the new pane gets. `None` uses the
    /// default of the launcher.
    pub ratio: Option<u8>,
}

/// A run of pr-agent: one tool on one pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentJob {
    pub key: PrKey,
    /// The head commit of the pull request when the run starts.
    pub head_sha: String,
    /// The local checkout of the repo. pr-agent runs in it.
    pub repo_path: PathBuf,
    pub tool: Tool,
}

/// A sync failure, mapped from the adapter errors by the workers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SyncError {
    Auth,
    RateLimited { until: DateTime<Utc> },
    Offline,
    Other(String),
}

/// The GitHub rate limit after a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RateInfo {
    pub remaining: u32,
    pub reset_at: DateTime<Utc>,
    pub last_cost: u32,
}

/// The input that the user gave to an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ActionInput {
    None,
    Confirmed,
    Text(String),
}

/// The content of a confirm popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfirmSpec {
    pub title: String,
    pub lines: Vec<String>,
    /// True for a danger border.
    pub danger: bool,
}

/// The content of a prompt popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PromptSpec {
    pub title: String,
    pub placeholder: String,
}

/// The content of a plain popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PopupSpec {
    pub title: String,
    pub lines: Vec<String>,
}

/// The state of an async merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MergeProgress {
    Pending,
    Merged { sha: String },
    Enqueued,
    Failed(String),
}

/// A fire-and-forget write to the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Persist {
    UiState { key: String, value: String },
}

/// A UI event that `update` pushes for the transitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum UiEvent {
    TabChanged {
        from: usize,
        to: usize,
    },
    OverlayOpened(OverlayKind),
    OverlayClosed(OverlayKind),
    ScrollTo {
        tab: &'static str,
        from: usize,
        to: usize,
    },
    StatusExpired,
    /// The active pane tab of the popup changed. `from` and `to` are pane
    /// registry indexes.
    PaneTabChanged {
        from: usize,
        to: usize,
    },
}

/// An event from a running action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ActionEvent {
    Progress(String),
    /// Lines to append to the output popup.
    Output(Vec<String>),
    Popup(PopupSpec),
    MergeStatus(MergeProgress),
    Done(Result<String, String>),
}

/// A message into `app`. Each variant has one producer.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Msg {
    Key(KeyEvent),
    /// A mouse event at the cell `x`, `y`. The reader sends `hit: None`,
    /// and `run` sets the hit from the last drawn frame.
    Mouse {
        kind: MouseKind,
        x: u16,
        y: u16,
        hit: Option<Hit>,
    },
    Resize(Size),
    Tick,
    TransitionsDone,
    Signal,
    StartupDone {
        login: String,
        cwd_repo: Option<Repo>,
    },
    /// A sync of `scope` starts: a requested one or a timed one. Its pages
    /// and its end follow in `PrsSynced`, or a `SyncFailed` ends it.
    SyncStarted {
        scope: Scope,
    },
    /// One page of a sync, or the end of a sync. The pages of a sync
    /// arrive one by one, so the list fills while the sync runs.
    PrsSynced {
        scope: Scope,
        /// The pull requests of one page. The message that ends the sync
        /// has none.
        prs: Vec<PullRequest>,
        /// Every pull request of the sync. Only the end of a full sync
        /// that read every page has it.
        seen: Option<Vec<PrKey>>,
        /// The start time of the first page of the sync.
        started_at: DateTime<Utc>,
        /// True on the message that ends the sync.
        last: bool,
    },
    PrRefreshed(Box<PullRequest>),
    SyncFailed {
        /// `sync`, `ci`, `detail` or `prefetch`.
        source: &'static str,
        scope: Option<Scope>,
        err: SyncError,
    },
    Rate(RateInfo),
    CiUpdated {
        /// The key, the head oid and the CI state of each pull request.
        states: Vec<(PrKey, String, Observed<CiState>)>,
        selected_checks: Option<(PrKey, Observed<Vec<Check>>)>,
    },
    /// The detail worker starts to read the conversation of a pull
    /// request from GitHub. A `DetailLoaded` of the same key follows.
    DetailLoading(PrKey),
    DetailLoaded(PrKey, Result<Observed<PrDetail>, SyncError>),
    /// The detail worker starts to read the changed files of a pull
    /// request from GitHub. A `FilesLoaded` of the same key follows.
    FilesLoading(PrKey),
    FilesLoaded(PrKey, Result<Observed<PrFiles>, SyncError>),
    JjScanned {
        repo: RepoKey,
        state: Observed<LocalState>,
    },
    PrefetchDone(PrKey, Result<(), String>),
    /// The progress of a prefetch batch: `done` of `total` pull requests.
    /// `done == total` ends the batch.
    PrefetchBatch {
        done: usize,
        total: usize,
    },
    ActionEvent {
        id: &'static str,
        key: TargetKey,
        event: ActionEvent,
    },
    PaneStarted {
        id: &'static str,
        key: TargetKey,
    },
    /// The runtime started the pr-agent run of `kind` on `key`.
    AgentStarted {
        key: PrKey,
        kind: ToolKind,
    },
    /// The pr-agent run of `kind` on `key` ended: the new record, or the
    /// error.
    AgentFinished {
        key: PrKey,
        kind: ToolKind,
        result: Result<Record, String>,
    },
    /// The result files of `key`, read from disk.
    AgentLoaded {
        key: PrKey,
        results: AgentResults,
    },
    PaneEnded {
        id: &'static str,
        key: TargetKey,
    },
    WorkerDied {
        name: &'static str,
        restart_in: Duration,
    },
}

/// A command out of `app`. Each variant has one executor.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Cmd {
    SetInterest(Interest),
    SetSession(Session),
    SyncNow(Scope),
    RefreshPr(Vec<PrRef>),
    /// Reads one pull request by number.
    LoadPr(PrKey),
    ScanJj(RepoKey),
    RunAction {
        id: &'static str,
        selection: Box<Selection>,
        input: ActionInput,
    },
    RunTerminal {
        id: &'static str,
        key: TargetKey,
        request: LaunchRequest,
    },
    FocusPane {
        id: &'static str,
        key: TargetKey,
    },
    CancelAction {
        id: &'static str,
        key: TargetKey,
    },
    /// Runs pr-agent. An `Ask` with an empty question never leaves `app`:
    /// a pane sends it to open the question prompt.
    RunAgent(AgentJob),
    /// Reads the result files of a pull request.
    LoadAgent(PrKey),
    Persist(Persist),
    Copy(String),
    OpenUrl(String),
    Quit,
}
