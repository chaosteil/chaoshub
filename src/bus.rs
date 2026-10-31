//! Contracts between the layers: `Msg`, `Cmd`, `Interest`, `Session`,
//! `Selection`, specs and errors. Plain data only.
//!
//! `bus` imports only `domain` and the crossterm key types. No adapter type
//! appears here.
#![allow(
    dead_code,
    reason = "the outer layers use the contracts in later phases"
)]

use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::domain::check::Check;
use crate::domain::detail::PrDetail;
use crate::domain::keys::{PrKey, RepoKey};
use crate::domain::local::{LocalChange, LocalMatch, LocalState};
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
    Local { repo: RepoKey, change_id: String },
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
    Local {
        change: LocalChange,
        repo: Repo,
    },
}

impl Target {
    /// The key of the target.
    pub(crate) fn key(&self) -> TargetKey {
        match self {
            Target::Pr { pr, .. } => TargetKey::Pr(pr.key.clone()),
            Target::Local { change, repo } => TargetKey::Local {
                repo: repo.key.clone(),
                change_id: change.change_id.clone(),
            },
        }
    }

    /// The repo of the target.
    pub(crate) fn repo(&self) -> &RepoKey {
        match self {
            Target::Pr { repo, .. } | Target::Local { repo, .. } => &repo.key,
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
    Resize(Size),
    Tick,
    TransitionsDone,
    Signal,
    StartupDone {
        login: String,
        cwd_repo: Option<Repo>,
    },
    PrsSynced {
        scope: Scope,
        prs: Vec<PullRequest>,
        gone: Option<Vec<PrKey>>,
        started_at: DateTime<Utc>,
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
    DetailLoaded(PrKey, Result<Observed<PrDetail>, SyncError>),
    JjScanned {
        repo: RepoKey,
        state: Observed<LocalState>,
    },
    PrefetchDone(PrKey, Result<(), String>),
    ActionEvent {
        id: &'static str,
        key: TargetKey,
        event: ActionEvent,
    },
    PaneStarted {
        id: &'static str,
        key: TargetKey,
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
    Persist(Persist),
    Copy(String),
    OpenUrl(String),
    Quit,
}
