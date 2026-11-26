//! The runtime that executes commands.
//!
//! Each `Cmd` has one executor. This runtime sends the interest and the
//! session to the workers through watch channels, sends sync, detail, jj
//! and highlight requests through their request channels, writes `ui_state` through
//! the store handle, and runs the clipboard and the browser on blocking
//! threads. It runs the action tasks in a `JoinSet`, each with a child
//! token of the root token, and sends `ActionEvent::Done` with the result
//! of each task. With a launcher, it runs terminal commands in detached
//! panes and keeps their handles.

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::actions::{Action, Services};
use crate::agent::run::Job;
use crate::agent::store::Store;
use crate::bus::{
    ActionEvent, ActionInput, AgentJob, Cmd, DetailReq, HighlightReq, Interest, JjReq,
    LaunchRequest, Msg, PaletteAsk, PaletteReply, Selection, Session, SyncReq, TargetKey,
};
use crate::config::schema::AgentConfig;
use crate::domain::keys::PrKey;
use crate::domain::resume::{PaneResume, TOKEN_ENV};
use crate::github::{GithubClient, TokenSource};
use crate::launch::palette::Palette;
use crate::launch::{LaunchHandle, Launcher};
use crate::store::handle::StoreHandle;
use crate::vcs::lock::RepoLocks;

/// The longest wait on quit for the cancelled tasks to end.
const QUIT_WAIT: Duration = Duration::from_millis(500);
/// The interval of the `alive` poll of a pane, the fallback of `wait`.
const ALIVE_POLL: Duration = Duration::from_secs(5);
/// The number of failed `alive` polls in a row that end a pane.
const ALIVE_MISSES: u8 = 2;

/// The key of a running task: the action id and the target.
type TaskKey = (&'static str, TargetKey);

/// The handles of the open panes, shared with their runner tasks.
type PaneHandles = Arc<Mutex<HashMap<TaskKey, LaunchHandle>>>;

/// The channels to the workers and the store.
#[derive(Debug)]
pub(crate) struct Senders {
    pub interest: watch::Sender<Interest>,
    pub session: watch::Sender<Session>,
    pub sync: mpsc::Sender<SyncReq>,
    pub detail: mpsc::Sender<DetailReq>,
    pub jj: mpsc::Sender<JjReq>,
    pub highlight: mpsc::Sender<HighlightReq>,
    pub store: StoreHandle,
}

/// What the action tasks need: the registry and the parts of
/// `Services`.
pub(crate) struct ActionEnv {
    /// The actions that `Cmd::RunAction` names by id.
    pub registry: &'static [Box<dyn Action>],
    pub client: Arc<GithubClient>,
    pub locks: RepoLocks,
    /// Sends `Msg::ActionEvent`, `Msg::PaneStarted` and `Msg::PaneEnded`
    /// to `app`.
    pub tx: mpsc::Sender<Msg>,
    /// The launcher of detached panes. `None` when the event loop runs
    /// terminal commands in the foreground.
    pub launcher: Option<Arc<dyn Launcher>>,
    /// What the agent runs need. `None` without a state directory.
    pub agent: Option<Arc<AgentEnv>>,
    /// The directory of the codeview comment files. `None` when the home
    /// directory is unknown.
    pub codeview_dir: Option<PathBuf>,
    /// The command palette of Rex for `Cmd::AskPalette`. `None` outside
    /// Rex.
    pub palette: Option<Arc<dyn Palette>>,
}

/// What the agent runs need: the `[agent]` section, the result files
/// and the GitHub token of pr-agent.
#[derive(Debug)]
pub(crate) struct AgentEnv {
    pub config: AgentConfig,
    pub store: Store,
    pub token: TokenSource,
}

impl std::fmt::Debug for ActionEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionEnv")
            .field("actions", &self.registry.len())
            .field("launcher", &self.launcher.is_some())
            .field("palette", &self.palette.is_some())
            .finish_non_exhaustive()
    }
}

/// Executes the commands of `update`.
#[derive(Debug)]
pub(crate) struct Runtime {
    /// The tokio runtime of the workers and the blocking tasks.
    handle: Handle,
    /// Cancels every task on quit.
    root: CancellationToken,
    senders: Senders,
    env: ActionEnv,
    /// The running action tasks. Each returns its key and its serial.
    tasks: JoinSet<(TaskKey, u64)>,
    /// The serial and the token of the newest task of each key.
    tokens: HashMap<TaskKey, (u64, CancellationToken)>,
    /// The serial of the next task.
    next: u64,
    /// The handles of the open panes.
    panes: PaneHandles,
    /// The interval of the `alive` poll of a pane.
    alive_poll: Duration,
    /// True after `Cmd::Quit`.
    quit: bool,
    /// True when a quit closes the open panes. The pinned mode leaves
    /// them open. The pane runners share the flag, so
    /// `keep_panes_on_quit` reaches the panes that run.
    close_panes_on_quit: Arc<AtomicBool>,
    /// The socket of the list process that started this PR view, from
    /// `CHAOSHUB_LIST_SOCKET`. `Cmd::TellList` sends to it.
    list: Option<PathBuf>,
    /// The token of the question in the command palette of Rex that
    /// waits for its answer. A cancel stops the question.
    palette_ask: Option<CancellationToken>,
}

impl Runtime {
    /// Creates the runtime on the tokio runtime of `handle`. When
    /// `close_panes_on_quit` is false, a quit ends the pane runners and
    /// leaves the panes open. `Cmd::CancelAction` closes a pane in both
    /// cases.
    pub(crate) fn new(
        handle: Handle,
        senders: Senders,
        env: ActionEnv,
        close_panes_on_quit: bool,
    ) -> Self {
        Self {
            handle,
            root: CancellationToken::new(),
            senders,
            env,
            tasks: JoinSet::new(),
            tokens: HashMap::new(),
            next: 0,
            panes: Arc::default(),
            alive_poll: ALIVE_POLL,
            quit: false,
            close_panes_on_quit: Arc::new(AtomicBool::new(close_panes_on_quit)),
            list: None,
            palette_ask: None,
        }
    }

    /// Sends `Cmd::TellList` to the list socket at `path`.
    pub(crate) fn with_list(mut self, path: Option<PathBuf>) -> Self {
        self.list = path;
        self
    }

    /// Makes the next quit leave the open panes open, so Rex can restore
    /// them with the session. `Cmd::CancelAction` still closes a pane.
    #[allow(dead_code, reason = "the resume wiring in run uses it")]
    pub(crate) fn keep_panes_on_quit(&self) {
        self.close_panes_on_quit.store(false, Ordering::SeqCst);
    }

    /// The handles of the open panes of the action `id`, for the resume
    /// state.
    #[allow(dead_code, reason = "the resume wiring in run uses it")]
    pub(crate) fn pane_handles(&self, id: &str) -> Vec<PaneResume> {
        lock(&self.panes)
            .iter()
            .filter(|((pane_id, _), _)| *pane_id == id)
            .map(|((_, TargetKey::Pr(pr)), handle)| PaneResume {
                pr: pr.clone(),
                block_id: handle.block_id.clone(),
                label: handle.label.clone(),
            })
            .collect()
    }

    /// The root token. Quit cancels it.
    #[cfg(test)]
    pub(crate) fn root(&self) -> &CancellationToken {
        &self.root
    }

    /// Executes one command.
    pub(crate) fn exec(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::SetInterest(interest) => {
                self.senders.interest.send_replace(interest);
            }
            Cmd::SetSession(session) => {
                self.senders.session.send_replace(session);
            }
            Cmd::SyncNow(scope) => self.request(&self.senders.sync, SyncReq::Full(scope)),
            Cmd::RefreshPr(refs) => self.request(&self.senders.sync, SyncReq::Refresh(refs)),
            Cmd::LoadPr(key) => self.request(&self.senders.sync, SyncReq::Load(key)),
            Cmd::ReloadDetail(refs) => {
                self.request(&self.senders.detail, DetailReq::Reload(refs));
            }
            Cmd::ScanJj(repo) => self.request(&self.senders.jj, JjReq::Scan(repo)),
            Cmd::Highlight(req) => self.request(&self.senders.highlight, req),
            Cmd::Persist(persist) => self.senders.store.try_persist(persist),
            Cmd::Copy(text) => {
                self.handle.spawn_blocking(move || copy(&text));
            }
            Cmd::ReadClipboard => {
                let tx = self.env.tx.clone();
                self.handle.spawn(async move {
                    let text = tokio::task::spawn_blocking(crate::clipboard::paste)
                        .await
                        .ok()
                        .flatten();
                    send(&tx, Msg::ClipboardRead(text)).await;
                });
            }
            Cmd::OpenUrl(url) => {
                self.handle.spawn_blocking(move || {
                    if let Err(err) = open_url(&url) {
                        tracing::warn!(%err, url, "cannot open the url");
                    }
                });
            }
            Cmd::Quit => self.quit(),
            Cmd::RunAction {
                id,
                selection,
                input,
            } => self.run_action(id, *selection, input),
            Cmd::CancelAction { id, key } => {
                if let Some((_, token)) = self.tokens.get(&(id, key)) {
                    token.cancel();
                }
            }
            Cmd::RunTerminal { id, key, request } => self.run_terminal(id, key, request),
            Cmd::AdoptPane {
                id,
                key,
                block_id,
                label,
            } => self.adopt_pane(id, key, block_id, label),
            Cmd::FocusPane { id, key } => self.focus(id, key),
            Cmd::RunAgent(job) => self.run_agent(job),
            Cmd::LoadAgent(key) => self.load_agent(key),
            Cmd::LoadCodeview(key) => self.load_codeview(key),
            Cmd::TellList(event) => {
                if let Some(path) = &self.list {
                    crate::ipc::send(path, &event);
                }
            }
            Cmd::EditText(_) => tracing::warn!("the event loop runs the editor"),
            Cmd::AskPalette { serial, ask } => self.ask_palette(serial, *ask),
            Cmd::CancelPalette => {
                if let Some(token) = self.palette_ask.take() {
                    token.cancel();
                }
            }
        }
    }

    /// Asks `ask` in the command palette of Rex in a task with a child
    /// token of the root token, and sends `PaletteAnswered` with the
    /// reply. A cancel of the token drops the question, and so stops the
    /// rex process, and sends nothing. A new question stops the old one.
    /// Without a palette, the reply is a fallback.
    fn ask_palette(&mut self, serial: u64, ask: PaletteAsk) {
        if let Some(token) = self.palette_ask.take() {
            token.cancel();
        }
        let tx = self.env.tx.clone();
        let Some(palette) = self.env.palette.clone() else {
            self.handle.spawn(async move {
                let reply = PaletteReply::Fallback("no Rex palette".to_owned());
                send(&tx, Msg::PaletteAnswered { serial, reply }).await;
            });
            return;
        };
        let token = self.root.child_token();
        self.palette_ask = Some(token.clone());
        self.handle.spawn(async move {
            tokio::select! {
                reply = palette.ask(&ask) => {
                    send(&tx, Msg::PaletteAnswered { serial, reply }).await;
                }
                () = token.cancelled() => {}
            }
        });
    }

    /// Spawns an agent run with a child token of the root token. The
    /// task sends `AgentStarted`, then `AgentFinished` with the record or
    /// the error. A cancel kills the process.
    fn run_agent(&mut self, job: AgentJob) {
        self.reap();
        let id = crate::agent::ID;
        let target = TargetKey::Pr(job.key.clone());
        let (serial, cancel) = self.track(id, target.clone());
        let agent = self.env.agent.clone();
        let locks = self.env.locks.clone();
        let tx = self.env.tx.clone();
        self.tasks.spawn_on(
            async move {
                let kind = job.tool.kind();
                let key = job.key.clone();
                send(
                    &tx,
                    Msg::AgentStarted {
                        key: key.clone(),
                        kind,
                    },
                )
                .await;
                let result = match agent {
                    Some(agent) => {
                        let job = Job {
                            key: job.key,
                            head_sha: job.head_sha,
                            repo_path: job.repo_path,
                            base_ref: job.base_ref,
                            context: job.context,
                            tool: job.tool,
                        };
                        tokio::select! {
                            result = crate::agent::run::run(
                                &job, &agent.config, &agent.store, &agent.token, &locks, &tx,
                            ) => result,
                            () = cancel.cancelled() => Err("cancelled".to_owned()),
                        }
                    }
                    None => Err("no state directory for the results".to_owned()),
                };
                send(&tx, Msg::AgentFinished { key, kind, result }).await;
                ((id, target), serial)
            },
            &self.handle,
        );
    }

    /// Reads the result files of `key` and sends `AgentLoaded`.
    fn load_agent(&self, key: PrKey) {
        let Some(agent) = self.env.agent.clone() else {
            return;
        };
        let tx = self.env.tx.clone();
        self.handle.spawn(async move {
            let results = agent.store.load(&key);
            send(&tx, Msg::AgentLoaded { key, results }).await;
        });
    }

    /// Reads the codeview comment files of `key` on a blocking thread and
    /// sends `CodeviewLoaded`. It reads nothing without the comment
    /// directory or without a local path of the repo in the session.
    fn load_codeview(&self, key: PrKey) {
        let Some(dir) = self.env.codeview_dir.clone() else {
            return;
        };
        let repo = self
            .senders
            .session
            .borrow()
            .repos
            .iter()
            .find(|repo| repo.key == key.repo)
            .and_then(|repo| repo.path.clone());
        let Some(repo) = repo else {
            return;
        };
        let tx = self.env.tx.clone();
        let handle = self.handle.clone();
        self.handle.spawn(async move {
            let number = key.number;
            let comments = handle
                .spawn_blocking(move || crate::codeview::store::load(&dir, &repo, number))
                .await
                .unwrap_or_else(|err| Err(err.to_string()));
            send(&tx, Msg::CodeviewLoaded { key, comments }).await;
        });
    }

    /// Spawns the runner of a pane: it launches `request`, sends
    /// `PaneStarted`, waits for the pane to end and sends `PaneEnded`. A
    /// cancel of its token closes the pane. Without a launcher the event
    /// loop runs the command in the foreground, so this logs only.
    fn run_terminal(&mut self, id: &'static str, key: TargetKey, request: LaunchRequest) {
        let Some(launcher) = self.env.launcher.clone() else {
            tracing::warn!(id, "no launcher for a pane");
            return;
        };
        let (pane, serial) = self.pane_run(launcher, id, key);
        self.tasks.spawn_on(
            async move {
                pane.run(&request).await;
                ((pane.id, pane.key), serial)
            },
            &self.handle,
        );
    }

    /// Spawns the runner of a pane that runs already in the Rex block
    /// `block_id`, for example a pane that Rex restored with the session.
    /// When the block is gone, it sends `PaneEnded` at once. Else it
    /// watches the pane as `run_terminal` does. Without a launcher, it
    /// sends `PaneEnded` at once.
    fn adopt_pane(&mut self, id: &'static str, key: TargetKey, block_id: String, label: String) {
        let Some(launcher) = self.env.launcher.clone() else {
            tracing::debug!(id, "no launcher to adopt a pane");
            let tx = self.env.tx.clone();
            self.handle.spawn(async move {
                send(&tx, Msg::PaneEnded { id, key }).await;
            });
            return;
        };
        let (pane, serial) = self.pane_run(launcher, id, key);
        let handle = LaunchHandle { label, block_id };
        self.tasks.spawn_on(
            async move {
                pane.adopt(handle).await;
                ((pane.id, pane.key), serial)
            },
            &self.handle,
        );
    }

    /// A new runner of the pane of `(id, key)`, and the serial of its
    /// task.
    fn pane_run(
        &mut self,
        launcher: Arc<dyn Launcher>,
        id: &'static str,
        key: TargetKey,
    ) -> (PaneRun, u64) {
        self.reap();
        let (serial, cancel) = self.track(id, key.clone());
        let pane = PaneRun {
            launcher,
            id,
            key,
            cancel,
            tx: self.env.tx.clone(),
            panes: Arc::clone(&self.panes),
            alive_poll: self.alive_poll,
            quit: self.root.clone(),
            close_on_quit: Arc::clone(&self.close_panes_on_quit),
        };
        (pane, serial)
    }

    /// Moves the focus to the open pane of `(id, key)`.
    fn focus(&self, id: &'static str, key: TargetKey) {
        let handle = lock(&self.panes).get(&(id, key)).cloned();
        let (Some(launcher), Some(handle)) = (self.env.launcher.clone(), handle) else {
            tracing::debug!(id, "no open pane to focus");
            return;
        };
        self.handle.spawn(async move {
            if let Err(err) = launcher.focus(&handle).await {
                tracing::warn!(%err, label = handle.label, "cannot focus the pane");
            }
        });
    }

    /// Spawns the task of the action `id` with a child token of the root
    /// token. The task sends `ActionEvent::Done` with the result, or with
    /// `Err("cancelled")` when its token is cancelled first.
    fn run_action(&mut self, id: &'static str, selection: Selection, input: ActionInput) {
        self.reap();
        let Some(action) = self.env.registry.iter().find(|action| action.id() == id) else {
            tracing::warn!(id, "no action with this id");
            return;
        };
        let key = selection.target.key();
        let parts = action.pending(&selection, &input);
        let (serial, cancel) = self.track(id, key.clone());
        let svc = Services {
            client: Arc::clone(&self.env.client),
            locks: self.env.locks.clone(),
            cancel: cancel.clone(),
            tx: self.env.tx.clone(),
        };
        let task = action.execute(selection, input, svc);
        let tx = self.env.tx.clone();
        self.tasks.spawn_on(
            async move {
                let result = tokio::select! {
                    result = task => result,
                    () = cancel.cancelled() => Err("cancelled".to_owned()),
                };
                // The parts go first, so `app` ends the run of this
                // result, also when the runs end in another order.
                if !parts.is_empty() {
                    let parts = Msg::ActionEvent {
                        id,
                        key: key.clone(),
                        event: ActionEvent::Parts(parts),
                    };
                    if tx.send(parts).await.is_err() {
                        tracing::debug!(id, "app gone, action result dropped");
                    }
                }
                let done = Msg::ActionEvent {
                    id,
                    key: key.clone(),
                    event: ActionEvent::Done(result),
                };
                if tx.send(done).await.is_err() {
                    tracing::debug!(id, "app gone, action result dropped");
                }
                ((id, key), serial)
            },
            &self.handle,
        );
    }

    /// A new child token of the root token for the task of `(id, key)`,
    /// and its serial.
    fn track(&mut self, id: &'static str, key: TargetKey) -> (u64, CancellationToken) {
        let serial = self.next;
        self.next += 1;
        let token = self.root.child_token();
        self.tokens.insert((id, key), (serial, token.clone()));
        (serial, token)
    }

    /// Removes the finished tasks and their tokens.
    fn reap(&mut self) {
        while let Some(joined) = self.tasks.try_join_next() {
            match joined {
                Ok((key, serial)) => {
                    if self.tokens.get(&key).is_some_and(|(s, _)| *s == serial) {
                        self.tokens.remove(&key);
                    }
                }
                Err(err) => tracing::error!(%err, "an action task failed"),
            }
        }
    }

    /// Cancels the root token, so every task ends, and waits briefly for
    /// the tasks.
    fn quit(&mut self) {
        self.root.cancel();
        self.quit = true;
        if self.tasks.is_empty() {
            return;
        }
        let tasks = &mut self.tasks;
        let drained = self.handle.block_on(async {
            tokio::time::timeout(QUIT_WAIT, async {
                while tasks.join_next().await.is_some() {}
            })
            .await
        });
        if drained.is_err() {
            tracing::warn!(left = tasks.len(), "action tasks still run at quit");
        }
    }

    /// The number of running action tasks.
    #[cfg(test)]
    fn running(&mut self) -> usize {
        self.reap();
        self.tasks.len()
    }

    /// Sends a request to a worker. When the channel is full, a task
    /// waits for room, so no request is lost.
    fn request<T: Send + std::fmt::Debug + 'static>(&self, tx: &mpsc::Sender<T>, req: T) {
        match tx.try_send(req) {
            Ok(()) => {}
            Err(TrySendError::Full(req)) => {
                let tx = tx.clone();
                self.handle.spawn(async move {
                    if let Err(err) = tx.send(req).await {
                        tracing::warn!(req = ?err.0, "worker stopped, request dropped");
                    }
                });
            }
            Err(TrySendError::Closed(req)) => {
                tracing::warn!(?req, "worker stopped, request dropped");
            }
        }
    }

    /// True after `Cmd::Quit`. The event loop then stops.
    pub(crate) fn quitting(&self) -> bool {
        self.quit
    }
}

/// Sends `msg` to `app`. A closed channel means that `app` is gone.
async fn send(tx: &mpsc::Sender<Msg>, msg: Msg) {
    if tx.send(msg).await.is_err() {
        tracing::debug!("app is gone, agent message dropped");
    }
}

/// The runner of one pane.
struct PaneRun {
    launcher: Arc<dyn Launcher>,
    id: &'static str,
    key: TargetKey,
    cancel: CancellationToken,
    tx: mpsc::Sender<Msg>,
    panes: PaneHandles,
    alive_poll: Duration,
    /// The root token: a quit cancels it.
    quit: CancellationToken,
    /// True when a quit closes the pane. The runtime shares it.
    close_on_quit: Arc<AtomicBool>,
}

impl PaneRun {
    /// Launches the pane and follows it (see `follow`). A failed launch
    /// sends `Done` with the error only: no `PaneStarted` came, so no
    /// `PaneEnded` follows, and the follow-up runs once.
    async fn run(&self, request: &LaunchRequest) {
        if let Some(handle) = self.launch(request).await {
            self.follow(handle).await;
        }
    }

    /// Launches the pane. `None` after a failed launch, which sends
    /// `Done` with the error.
    async fn launch(&self, request: &LaunchRequest) -> Option<LaunchHandle> {
        match self.launcher.launch(request).await {
            Ok(handle) => Some(handle),
            Err(err) => {
                tracing::warn!(%err, id = self.id, "cannot open the pane");
                self.send(Msg::ActionEvent {
                    id: self.id,
                    key: self.key.clone(),
                    event: ActionEvent::Done(Err(format!("cannot open the pane: {err}"))),
                })
                .await;
                None
            }
        }
    }

    /// Follows a pane that runs already. When `alive` is false, the pane
    /// is gone: it sends `PaneEnded` at once. Else it follows the pane
    /// as a launched pane.
    async fn adopt(&self, handle: LaunchHandle) {
        if !self.launcher.alive(&handle).await {
            tracing::debug!(block = handle.block_id, "the adopted pane is gone");
            self.send(Msg::PaneEnded {
                id: self.id,
                key: self.key.clone(),
            })
            .await;
            return;
        }
        self.follow(handle).await;
    }

    /// Sends `PaneStarted`, watches the pane until it ends or the token
    /// is cancelled, and closes it on a cancel. A quit closes it only
    /// when `close_on_quit` is true. `PaneEnded` comes last.
    async fn follow(&self, handle: LaunchHandle) {
        lock(&self.panes).insert((self.id, self.key.clone()), handle.clone());
        self.send(Msg::PaneStarted {
            id: self.id,
            key: self.key.clone(),
        })
        .await;
        self.watch(&handle).await;
        let closes = self.close_on_quit.load(Ordering::SeqCst) || !self.quit.is_cancelled();
        if self.cancel.is_cancelled()
            && closes
            && let Err(err) = self.launcher.close(&handle).await
        {
            tracing::warn!(%err, label = handle.label, "cannot close the pane");
        }
        lock(&self.panes).remove(&(self.id, self.key.clone()));
        self.send(Msg::PaneEnded {
            id: self.id,
            key: self.key.clone(),
        })
        .await;
    }

    /// Returns when `wait` returns, when `alive` fails `ALIVE_MISSES`
    /// times in a row, or when the token is cancelled. A failed `wait`
    /// leaves the `alive` poll as the only watch.
    async fn watch(&self, handle: &LaunchHandle) {
        let mut wait = self.launcher.wait(handle);
        let mut waiting = true;
        let start = tokio::time::Instant::now() + self.alive_poll;
        let mut poll = tokio::time::interval_at(start, self.alive_poll);
        let mut misses = 0;
        loop {
            tokio::select! {
                code = &mut wait, if waiting => match code {
                    Ok(code) => {
                        tracing::debug!(code, label = handle.label, "the pane exited");
                        return;
                    }
                    Err(err) => {
                        tracing::warn!(%err, label = handle.label, "cannot wait for the pane");
                        waiting = false;
                    }
                },
                _ = poll.tick() => {
                    if self.launcher.alive(handle).await {
                        misses = 0;
                    } else {
                        misses += 1;
                        if misses >= ALIVE_MISSES {
                            tracing::debug!(label = handle.label, "the pane is gone");
                            return;
                        }
                    }
                }
                () = self.cancel.cancelled() => return,
            }
        }
    }

    async fn send(&self, msg: Msg) {
        if self.tx.send(msg).await.is_err() {
            tracing::debug!(id = self.id, "app gone, pane event dropped");
        }
    }
}

/// Locks the pane handles. A panic in another holder does not stop the
/// runtime.
fn lock(panes: &PaneHandles) -> std::sync::MutexGuard<'_, HashMap<TaskKey, LaunchHandle>> {
    panes.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Puts `text` on the clipboard: the system clipboard, else OSC 52.
fn copy(text: &str) {
    match crate::clipboard::copy(text) {
        Ok(copied) => tracing::debug!(?copied, "copied to the clipboard"),
        Err(err) => tracing::warn!(%err, "cannot copy to the clipboard"),
    }
}

/// Opens `url` with the openers of `open::commands`, without
/// `OSC7577_TOKEN`. The first opener that ends with success wins.
fn open_url(url: &str) -> io::Result<()> {
    let mut last = io::Error::other("no opener for the url");
    for mut cmd in open::commands(url) {
        let status = cmd
            .env_remove(TOKEN_ENV)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match status {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => {
                last = io::Error::other(format!(
                    "{} ended with {status}",
                    cmd.get_program().display()
                ));
            }
            Err(err) => last = err,
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{Persist, Scope};
    use crate::domain::keys::RepoKey;
    use crate::store::handle::StoreOp;

    /// The runtime and the receiving ends of its channels.
    struct Fixture {
        runtime: Runtime,
        interest: watch::Receiver<Interest>,
        session: watch::Receiver<Session>,
        sync: mpsc::Receiver<SyncReq>,
        detail: mpsc::Receiver<DetailReq>,
        jj: mpsc::Receiver<JjReq>,
        highlight: mpsc::Receiver<HighlightReq>,
        store: mpsc::Receiver<StoreOp>,
        msgs: mpsc::Receiver<Msg>,
    }

    fn interest(tab: &'static str) -> Interest {
        Interest {
            tab,
            scope: None,
            wants_local: false,
            selected: None,
            visible: Vec::new(),
            show_hidden: false,
            ci_watch: Vec::new(),
        }
    }

    fn fixture(handle: Handle, capacity: usize) -> Fixture {
        fixture_with(handle, capacity, Vec::new())
    }

    /// The runtime with the actions of `registry`.
    fn fixture_with(handle: Handle, capacity: usize, registry: Vec<Box<dyn Action>>) -> Fixture {
        fixture_full(handle, capacity, registry, None)
    }

    /// The runtime with the actions of `registry` and `launcher`.
    fn fixture_full(
        handle: Handle,
        capacity: usize,
        registry: Vec<Box<dyn Action>>,
        launcher: Option<Arc<dyn Launcher>>,
    ) -> Fixture {
        let (interest_tx, interest) = watch::channel(interest("none"));
        let (session_tx, session) = watch::channel(Session {
            login: None,
            repos: Vec::new(),
        });
        let (sync_tx, sync) = mpsc::channel(capacity);
        let (detail_tx, detail) = mpsc::channel(capacity);
        let (jj_tx, jj) = mpsc::channel(capacity);
        let (highlight_tx, highlight) = mpsc::channel(capacity);
        let (store_tx, store) = mpsc::channel(4);
        let senders = Senders {
            interest: interest_tx,
            session: session_tx,
            sync: sync_tx,
            detail: detail_tx,
            jj: jj_tx,
            highlight: highlight_tx,
            store: StoreHandle::new(store_tx),
        };
        let (tx, msgs) = mpsc::channel(16);
        let client = GithubClient::new(
            crate::github::TokenSource::command("true", &[]),
            "http://127.0.0.1:9",
        )
        .unwrap();
        let env = ActionEnv {
            registry: Box::leak(Box::new(registry)),
            client: Arc::new(client),
            locks: RepoLocks::default(),
            tx,
            launcher,
            agent: None,
            codeview_dir: None,
            palette: None,
        };
        Fixture {
            runtime: Runtime::new(handle, senders, env, true),
            interest,
            session,
            sync,
            detail,
            jj,
            highlight,
            store,
            msgs,
        }
    }

    fn repo() -> RepoKey {
        RepoKey {
            owner: "o".into(),
            name: "r".into(),
        }
    }

    #[tokio::test]
    async fn commands_reach_their_channels() {
        let mut f = fixture(Handle::current(), 4);
        f.runtime.exec(Cmd::SetInterest(interest("mine")));
        assert_eq!(f.interest.borrow_and_update().tab, "mine");
        let session = Session {
            login: Some("me".into()),
            repos: Vec::new(),
        };
        f.runtime.exec(Cmd::SetSession(session.clone()));
        assert_eq!(*f.session.borrow_and_update(), session);
        f.runtime.exec(Cmd::SyncNow(Scope::Team));
        f.runtime.exec(Cmd::RefreshPr(Vec::new()));
        f.runtime.exec(Cmd::ReloadDetail(Vec::new()));
        f.runtime.exec(Cmd::ScanJj(repo()));
        let cancel = HighlightReq::Cancel {
            key: crate::domain::keys::PrKey {
                repo: repo(),
                number: 1,
            },
            path: "a.rs".into(),
        };
        f.runtime.exec(Cmd::Highlight(cancel.clone()));
        assert_eq!(f.sync.recv().await, Some(SyncReq::Full(Scope::Team)));
        assert_eq!(f.sync.recv().await, Some(SyncReq::Refresh(Vec::new())));
        assert_eq!(f.detail.recv().await, Some(DetailReq::Reload(Vec::new())));
        assert_eq!(f.jj.recv().await, Some(JjReq::Scan(repo())));
        assert_eq!(f.highlight.recv().await, Some(cancel));
        let persist = Persist::UiState {
            key: "filter.mine".into(),
            value: "pane".into(),
        };
        f.runtime.exec(Cmd::Persist(persist.clone()));
        assert!(matches!(f.store.recv().await, Some(StoreOp::Persist(p)) if p == persist));
        assert!(!f.runtime.quitting());
    }

    #[tokio::test]
    async fn a_full_request_channel_waits_instead_of_dropping() {
        let mut f = fixture(Handle::current(), 1);
        f.runtime.exec(Cmd::ScanJj(repo()));
        f.runtime.exec(Cmd::SyncNow(Scope::Mine));
        f.runtime.exec(Cmd::SyncNow(Scope::Team));
        assert_eq!(f.sync.recv().await, Some(SyncReq::Full(Scope::Mine)));
        assert_eq!(f.sync.recv().await, Some(SyncReq::Full(Scope::Team)));
        assert_eq!(f.jj.recv().await, Some(JjReq::Scan(repo())));
    }

    #[tokio::test]
    async fn tell_list_sends_to_the_list_socket_and_skips_without_one() {
        let event = crate::bus::PrViewEvent::PrChanged {
            pr: crate::domain::keys::PrKey {
                repo: repo(),
                number: 7,
            },
        };
        // Without a socket, nothing happens.
        let mut f = fixture(Handle::current(), 4);
        f.runtime.exec(Cmd::TellList(event.clone()));
        // A socket that no list listens on: the send fails and returns.
        let dir = tempfile::tempdir().unwrap();
        let mut f = fixture(Handle::current(), 4);
        f.runtime = f.runtime.with_list(Some(dir.path().join("list-1.sock")));
        f.runtime.exec(Cmd::TellList(event.clone()));
        // A list that listens gets the event.
        let (tx, rx) = std::sync::mpsc::channel();
        let listener =
            crate::ipc::Listener::bind(dir.path(), "1", move |event| tx.send(event).is_ok())
                .unwrap();
        f.runtime = f.runtime.with_list(Some(listener.path().to_path_buf()));
        f.runtime.exec(Cmd::TellList(event.clone()));
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(event));
    }

    #[test]
    fn quit_cancels_the_root_token() {
        let tokio = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut f = fixture(tokio.handle().clone(), 4);
        drop(f.sync);
        f.runtime.exec(Cmd::SyncNow(Scope::Mine));
        assert!(
            !f.runtime.quitting(),
            "a stopped worker does not stop the app"
        );
        f.runtime.exec(Cmd::Quit);
        assert!(f.runtime.quitting());
        assert!(f.runtime.root().is_cancelled());
    }

    /// An action whose task sends one progress line and returns, or
    /// waits for ever when `waits` is true.
    struct TestAction {
        id: &'static str,
        waits: bool,
    }

    impl Action for TestAction {
        fn id(&self) -> &'static str {
            self.id
        }

        fn label(&self) -> &'static str {
            self.id
        }

        fn default_key(&self) -> Option<crate::bus::KeyChord> {
            None
        }

        fn group(&self) -> crate::actions::Group {
            crate::actions::Group::Change
        }

        fn applies(&self, _sel: &Selection) -> crate::actions::Applicability {
            crate::actions::Applicability::Enabled
        }

        fn prepare(&self, _sel: &Selection) -> crate::actions::Prepare {
            crate::actions::Prepare::Run
        }

        fn execute(
            &self,
            sel: Selection,
            input: ActionInput,
            svc: Services,
        ) -> futures::future::BoxFuture<'static, Result<String, String>> {
            let (id, waits) = (self.id, self.waits);
            Box::pin(async move {
                if waits {
                    return futures::future::pending().await;
                }
                let event = Msg::ActionEvent {
                    id,
                    key: sel.target.key(),
                    event: ActionEvent::Progress(format!("{input:?}")),
                };
                svc.tx.send(event).await.map_err(|err| err.to_string())?;
                Ok(format!("{id} done"))
            })
        }
    }

    fn actions() -> Vec<Box<dyn Action>> {
        vec![
            Box::new(TestAction {
                id: "talks",
                waits: false,
            }),
            Box::new(TestAction {
                id: "waits",
                waits: true,
            }),
        ]
    }

    fn selection(number: u64) -> Selection {
        let pr = crate::app::testdata::pr("chaoshub", number, "One");
        Selection {
            target: crate::bus::Target::Pr {
                repo: crate::domain::repo::Repo {
                    key: pr.key.repo.clone(),
                    path: None,
                    settings: None,
                },
                pr,
                stack: Vec::new(),
                local: crate::domain::local::LocalMatch::None,
            },
            me: "chaosteil".into(),
            action_config: None,
        }
    }

    fn pr_key(number: u64) -> TargetKey {
        TargetKey::Pr(crate::app::testdata::pr_key("chaoshub", number))
    }

    fn run_action(id: &'static str, number: u64, input: ActionInput) -> Cmd {
        Cmd::RunAction {
            id,
            selection: Box::new(selection(number)),
            input,
        }
    }

    fn done(id: &'static str, number: u64, result: Result<&str, &str>) -> Msg {
        Msg::ActionEvent {
            id,
            key: pr_key(number),
            event: ActionEvent::Done(result.map(str::to_owned).map_err(str::to_owned)),
        }
    }

    /// A multi-thread tokio runtime, so `exec` can run on the test thread
    /// outside of it, like on the main thread of the app.
    fn tokio() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap()
    }

    /// Waits until `runtime` has `count` running tasks, for at most two
    /// seconds. A task ends just after its last message, so a test that
    /// read the last message can still count the task.
    fn settle(runtime: &mut Runtime, count: usize) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            let running = runtime.running();
            if running == count {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{running} tasks run after 2 s, not {count}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_run_action_sends_its_events_and_done() {
        let tokio = tokio();
        let mut f = fixture_with(tokio.handle().clone(), 4, actions());
        f.runtime
            .exec(run_action("talks", 1, ActionInput::Text("lgtm".into())));
        let progress = Msg::ActionEvent {
            id: "talks",
            key: pr_key(1),
            event: ActionEvent::Progress("Text(\"lgtm\")".into()),
        };
        assert_eq!(f.msgs.blocking_recv(), Some(progress));
        assert_eq!(
            f.msgs.blocking_recv(),
            Some(done("talks", 1, Ok("talks done")))
        );
        settle(&mut f.runtime, 0);
        f.runtime.exec(run_action("missing", 1, ActionInput::None));
        assert_eq!(f.runtime.running(), 0);
        assert!(f.runtime.tokens.is_empty(), "the finished task left");
    }

    #[test]
    fn cancel_action_ends_the_task_of_its_key_only() {
        let tokio = tokio();
        let mut f = fixture_with(tokio.handle().clone(), 4, actions());
        f.runtime.exec(run_action("waits", 1, ActionInput::None));
        f.runtime.exec(run_action("waits", 2, ActionInput::None));
        f.runtime.exec(Cmd::CancelAction {
            id: "waits",
            key: pr_key(1),
        });
        assert_eq!(
            f.msgs.blocking_recv(),
            Some(done("waits", 1, Err("cancelled")))
        );
        f.runtime.exec(Cmd::CancelAction {
            id: "talks",
            key: pr_key(2),
        });
        assert!(f.msgs.try_recv().is_err(), "another id is another task");
        settle(&mut f.runtime, 1);
        assert_eq!(f.runtime.tokens.len(), 1);
    }

    #[test]
    fn quit_cancels_every_task_and_waits_for_them() {
        let tokio = tokio();
        let mut f = fixture_with(tokio.handle().clone(), 4, actions());
        f.runtime.exec(run_action("waits", 1, ActionInput::None));
        f.runtime
            .exec(run_action("waits", 2, ActionInput::Confirmed));
        f.runtime.exec(Cmd::Quit);
        assert!(f.runtime.quitting());
        assert!(f.runtime.tasks.is_empty(), "quit waited for the tasks");
        let mut got = vec![
            f.msgs.blocking_recv().unwrap(),
            f.msgs.blocking_recv().unwrap(),
        ];
        got.sort_by_key(|msg| format!("{msg:?}"));
        assert_eq!(
            got,
            [
                done("waits", 1, Err("cancelled")),
                done("waits", 2, Err("cancelled"))
            ]
        );
    }

    /// A launcher that records its calls. `wait` returns when the test
    /// notifies `exit`, or fails at once when `wait_fails`; `alive` reads
    /// `alive`.
    #[derive(Default)]
    struct FakeLauncher {
        calls: Mutex<Vec<String>>,
        exit: tokio::sync::Notify,
        alive: std::sync::atomic::AtomicBool,
        launch_fails: bool,
        wait_fails: bool,
    }

    impl FakeLauncher {
        fn record(&self, call: String) {
            self.calls.lock().unwrap().push(call);
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn error() -> crate::launch::error::LaunchError {
            crate::launch::error::LaunchError::Spawn {
                program: "rex".into(),
                source: std::io::Error::other("fake"),
            }
        }
    }

    impl Launcher for FakeLauncher {
        fn launch(
            &self,
            req: &LaunchRequest,
        ) -> futures::future::BoxFuture<'_, Result<LaunchHandle, crate::launch::error::LaunchError>>
        {
            self.record(format!("launch {}", req.label));
            let label = req.label.clone();
            Box::pin(async move {
                if self.launch_fails {
                    return Err(Self::error());
                }
                Ok(LaunchHandle {
                    block_id: format!("block:{label}"),
                    label,
                })
            })
        }

        fn wait(
            &self,
            handle: &LaunchHandle,
        ) -> futures::future::BoxFuture<'_, Result<i32, crate::launch::error::LaunchError>>
        {
            self.record(format!("wait {}", handle.block_id));
            Box::pin(async move {
                if self.wait_fails {
                    return Err(Self::error());
                }
                self.exit.notified().await;
                Ok(0)
            })
        }

        fn alive(&self, handle: &LaunchHandle) -> futures::future::BoxFuture<'_, bool> {
            self.record(format!("alive {}", handle.block_id));
            Box::pin(async move { self.alive.load(std::sync::atomic::Ordering::SeqCst) })
        }

        fn focus(
            &self,
            handle: &LaunchHandle,
        ) -> futures::future::BoxFuture<'_, Result<(), crate::launch::error::LaunchError>> {
            self.record(format!("focus {}", handle.block_id));
            Box::pin(async { Ok(()) })
        }

        fn close(
            &self,
            handle: &LaunchHandle,
        ) -> futures::future::BoxFuture<'_, Result<(), crate::launch::error::LaunchError>> {
            self.record(format!("close {}", handle.block_id));
            Box::pin(async { Ok(()) })
        }
    }

    fn pane_fixture(tokio: &tokio::runtime::Runtime, launcher: &Arc<FakeLauncher>) -> Fixture {
        let launcher: Arc<dyn Launcher> = Arc::clone(launcher) as Arc<dyn Launcher>;
        let mut f = fixture_full(tokio.handle().clone(), 4, Vec::new(), Some(launcher));
        f.runtime.alive_poll = Duration::from_millis(20);
        f
    }

    fn run_terminal(number: u64) -> Cmd {
        Cmd::RunTerminal {
            id: "review",
            key: pr_key(number),
            request: LaunchRequest {
                argv: vec!["nvim".into()],
                cwd: "/code/chaoshub".into(),
                env: Vec::new(),
                label: format!("pane-{number}"),
                zoom: true,
                split: None,
                ratio: None,
            },
        }
    }

    fn started(number: u64) -> Msg {
        Msg::PaneStarted {
            id: "review",
            key: pr_key(number),
        }
    }

    fn ended(number: u64) -> Msg {
        Msg::PaneEnded {
            id: "review",
            key: pr_key(number),
        }
    }

    /// Waits until `check` is true, for at most two seconds.
    fn eventually(check: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !check() {
            assert!(std::time::Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_pane_starts_takes_the_focus_and_ends_with_its_command() {
        let tokio = tokio();
        let launcher = Arc::new(FakeLauncher {
            alive: true.into(),
            ..FakeLauncher::default()
        });
        let mut f = pane_fixture(&tokio, &launcher);
        f.runtime.exec(run_terminal(1));
        assert_eq!(f.msgs.blocking_recv(), Some(started(1)));
        // The pane task can start its wait after the `started` message.
        // Thus wait for it, so the focus comes after the wait.
        eventually(|| launcher.calls().contains(&"wait block:pane-1".to_owned()));
        f.runtime.exec(Cmd::FocusPane {
            id: "review",
            key: pr_key(1),
        });
        eventually(|| launcher.calls().contains(&"focus block:pane-1".to_owned()));
        launcher.exit.notify_one();
        assert_eq!(f.msgs.blocking_recv(), Some(ended(1)));
        let calls = launcher.calls();
        assert_eq!(calls[..2], ["launch pane-1", "wait block:pane-1"]);
        assert!(!calls.iter().any(|call| call.starts_with("close")));
        assert!(lock(&f.runtime.panes).is_empty());
        f.runtime.exec(Cmd::FocusPane {
            id: "review",
            key: pr_key(1),
        });
        // The task ends just after it sends `PaneEnded`. Thus wait for it.
        settle(&mut f.runtime, 0);
    }

    #[test]
    fn two_failed_alive_polls_end_a_pane() {
        let tokio = tokio();
        let launcher = Arc::new(FakeLauncher {
            wait_fails: true,
            ..FakeLauncher::default()
        });
        let mut f = pane_fixture(&tokio, &launcher);
        f.runtime.exec(run_terminal(2));
        assert_eq!(f.msgs.blocking_recv(), Some(started(2)));
        assert_eq!(f.msgs.blocking_recv(), Some(ended(2)));
        let alive = launcher
            .calls()
            .iter()
            .filter(|call| call.starts_with("alive"))
            .count();
        assert_eq!(alive, 2);
    }

    #[test]
    fn cancel_and_quit_close_the_pane() {
        let tokio = tokio();
        let launcher = Arc::new(FakeLauncher {
            alive: true.into(),
            ..FakeLauncher::default()
        });
        let mut f = pane_fixture(&tokio, &launcher);
        f.runtime.exec(run_terminal(1));
        f.runtime.exec(run_terminal(2));
        let mut first = vec![f.msgs.blocking_recv(), f.msgs.blocking_recv()];
        first.sort_by_key(|msg| format!("{msg:?}"));
        assert_eq!(first, [Some(started(1)), Some(started(2))]);
        f.runtime.exec(Cmd::CancelAction {
            id: "review",
            key: pr_key(1),
        });
        assert_eq!(f.msgs.blocking_recv(), Some(ended(1)));
        assert!(launcher.calls().contains(&"close block:pane-1".to_owned()));
        f.runtime.exec(Cmd::Quit);
        assert!(launcher.calls().contains(&"close block:pane-2".to_owned()));
        assert_eq!(f.msgs.blocking_recv(), Some(ended(2)));
    }

    #[test]
    fn a_quit_without_close_on_quit_leaves_the_pane_open() {
        let tokio = tokio();
        let launcher = Arc::new(FakeLauncher {
            alive: true.into(),
            ..FakeLauncher::default()
        });
        let mut f = pane_fixture(&tokio, &launcher);
        f.runtime.keep_panes_on_quit();
        f.runtime.exec(run_terminal(1));
        f.runtime.exec(run_terminal(2));
        let mut first = vec![f.msgs.blocking_recv(), f.msgs.blocking_recv()];
        first.sort_by_key(|msg| format!("{msg:?}"));
        assert_eq!(first, [Some(started(1)), Some(started(2))]);
        f.runtime.exec(Cmd::CancelAction {
            id: "review",
            key: pr_key(1),
        });
        assert_eq!(f.msgs.blocking_recv(), Some(ended(1)));
        assert!(
            launcher.calls().contains(&"close block:pane-1".to_owned()),
            "a cancel still closes the pane"
        );
        f.runtime.exec(Cmd::Quit);
        assert_eq!(f.msgs.blocking_recv(), Some(ended(2)));
        assert!(
            !launcher.calls().contains(&"close block:pane-2".to_owned()),
            "a quit leaves the pane open"
        );
    }

    fn adopt_pane(number: u64) -> Cmd {
        Cmd::AdoptPane {
            id: "review",
            key: pr_key(number),
            block_id: format!("block:old-{number}"),
            label: format!("pane-{number}"),
        }
    }

    #[test]
    fn an_adopted_pane_runs_like_a_launched_pane() {
        let tokio = tokio();
        let launcher = Arc::new(FakeLauncher {
            alive: true.into(),
            ..FakeLauncher::default()
        });
        let mut f = pane_fixture(&tokio, &launcher);
        f.runtime.exec(adopt_pane(1));
        assert_eq!(f.msgs.blocking_recv(), Some(started(1)));
        eventually(|| launcher.calls().contains(&"wait block:old-1".to_owned()));
        assert_eq!(
            launcher.calls()[..2],
            ["alive block:old-1", "wait block:old-1"]
        );
        assert_eq!(
            f.runtime.pane_handles("review"),
            [PaneResume {
                pr: crate::app::testdata::pr_key("chaoshub", 1),
                block_id: "block:old-1".into(),
                label: "pane-1".into(),
            }]
        );
        f.runtime.exec(Cmd::FocusPane {
            id: "review",
            key: pr_key(1),
        });
        eventually(|| launcher.calls().contains(&"focus block:old-1".to_owned()));
        f.runtime.exec(Cmd::Quit);
        assert!(launcher.calls().contains(&"close block:old-1".to_owned()));
        assert_eq!(f.msgs.blocking_recv(), Some(ended(1)));
    }

    #[test]
    fn a_gone_adopted_pane_ends_at_once() {
        let tokio = tokio();
        let launcher = Arc::new(FakeLauncher::default());
        let mut f = pane_fixture(&tokio, &launcher);
        f.runtime.exec(adopt_pane(2));
        assert_eq!(f.msgs.blocking_recv(), Some(ended(2)));
        settle(&mut f.runtime, 0);
        assert_eq!(launcher.calls(), ["alive block:old-2"]);
        assert_eq!(f.runtime.pane_handles("review"), []);
    }

    #[test]
    fn keep_panes_on_quit_leaves_the_panes_open() {
        let tokio = tokio();
        let launcher = Arc::new(FakeLauncher {
            alive: true.into(),
            ..FakeLauncher::default()
        });
        let mut f = pane_fixture(&tokio, &launcher);
        f.runtime.exec(run_terminal(1));
        f.runtime.exec(adopt_pane(2));
        let mut first = vec![f.msgs.blocking_recv(), f.msgs.blocking_recv()];
        first.sort_by_key(|msg| format!("{msg:?}"));
        assert_eq!(first, [Some(started(1)), Some(started(2))]);
        f.runtime.keep_panes_on_quit();
        f.runtime.exec(Cmd::Quit);
        let mut last = vec![f.msgs.blocking_recv(), f.msgs.blocking_recv()];
        last.sort_by_key(|msg| format!("{msg:?}"));
        assert_eq!(last, [Some(ended(1)), Some(ended(2))]);
        assert!(
            !launcher
                .calls()
                .iter()
                .any(|call| call.starts_with("close")),
            "{:?}",
            launcher.calls()
        );
    }

    #[test]
    fn a_failed_launch_sends_only_done_with_its_error() {
        let tokio = tokio();
        let launcher = Arc::new(FakeLauncher {
            launch_fails: true,
            ..FakeLauncher::default()
        });
        let mut f = pane_fixture(&tokio, &launcher);
        f.runtime.exec(run_terminal(3));
        assert_eq!(
            f.msgs.blocking_recv(),
            Some(Msg::ActionEvent {
                id: "review",
                key: pr_key(3),
                event: ActionEvent::Done(
                    Err("cannot open the pane: cannot start rex: fake".into())
                ),
            })
        );
        settle(&mut f.runtime, 0);
        assert!(
            f.msgs.try_recv().is_err(),
            "no PaneEnded after a failed launch"
        );
        assert_eq!(launcher.calls(), ["launch pane-3"]);
    }

    /// A palette that records its questions. It answers with `reply`
    /// when the test notifies `answer`. `asked` counts the questions,
    /// and `dropped` counts the questions that ended without a reply.
    struct FakePalette {
        reply: PaletteReply,
        answer: tokio::sync::Notify,
        asked: Mutex<Vec<PaletteAsk>>,
        dropped: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl FakePalette {
        fn new(reply: PaletteReply) -> Self {
            Self {
                reply,
                answer: tokio::sync::Notify::new(),
                asked: Mutex::new(Vec::new()),
                dropped: Arc::default(),
            }
        }

        fn asked(&self) -> usize {
            self.asked.lock().unwrap().len()
        }
    }

    /// Counts a drop of the future of a question before its reply.
    struct DropCount(Arc<std::sync::atomic::AtomicUsize>);

    impl Drop for DropCount {
        fn drop(&mut self) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    impl Palette for FakePalette {
        fn ask(&self, ask: &PaletteAsk) -> futures::future::BoxFuture<'_, PaletteReply> {
            self.asked.lock().unwrap().push(ask.clone());
            let guard = DropCount(Arc::clone(&self.dropped));
            Box::pin(async move {
                self.answer.notified().await;
                std::mem::forget(guard);
                self.reply.clone()
            })
        }
    }

    fn palette_fixture(tokio: &tokio::runtime::Runtime, palette: &Arc<FakePalette>) -> Fixture {
        let mut f = fixture(tokio.handle().clone(), 4);
        f.runtime.env.palette = Some(Arc::clone(palette) as Arc<dyn Palette>);
        f
    }

    fn ask_palette(serial: u64) -> Cmd {
        Cmd::AskPalette {
            serial,
            ask: Box::new(PaletteAsk::Input {
                title: "ask".into(),
                placeholder: String::new(),
                value: String::new(),
                confirm: None,
            }),
        }
    }

    #[test]
    fn a_palette_question_sends_its_reply() {
        let tokio = tokio();
        let palette = Arc::new(FakePalette::new(PaletteReply::Answer("yes".into())));
        let mut f = palette_fixture(&tokio, &palette);
        f.runtime.exec(ask_palette(3));
        eventually(|| palette.asked() == 1);
        palette.answer.notify_one();
        assert_eq!(
            f.msgs.blocking_recv(),
            Some(Msg::PaletteAnswered {
                serial: 3,
                reply: PaletteReply::Answer("yes".into()),
            })
        );
    }

    #[test]
    fn cancel_palette_drops_the_question_without_a_reply() {
        let tokio = tokio();
        let palette = Arc::new(FakePalette::new(PaletteReply::Cancelled));
        let mut f = palette_fixture(&tokio, &palette);
        f.runtime.exec(ask_palette(1));
        eventually(|| palette.asked() == 1);
        f.runtime.exec(Cmd::CancelPalette);
        eventually(|| palette.dropped.load(std::sync::atomic::Ordering::SeqCst) == 1);
        // A new question stops the old one too.
        f.runtime.exec(ask_palette(2));
        eventually(|| palette.asked() == 2);
        f.runtime.exec(ask_palette(3));
        eventually(|| palette.dropped.load(std::sync::atomic::Ordering::SeqCst) == 2);
        palette.answer.notify_one();
        assert_eq!(
            f.msgs.blocking_recv(),
            Some(Msg::PaletteAnswered {
                serial: 3,
                reply: PaletteReply::Cancelled,
            })
        );
        f.runtime.exec(Cmd::Quit);
        assert!(f.msgs.try_recv().is_err(), "no reply for 1 and 2");
    }

    #[test]
    fn without_a_palette_a_question_falls_back() {
        let tokio = tokio();
        let mut f = fixture(tokio.handle().clone(), 4);
        f.runtime.exec(ask_palette(5));
        assert!(matches!(
            f.msgs.blocking_recv(),
            Some(Msg::PaletteAnswered {
                serial: 5,
                reply: PaletteReply::Fallback(_),
            })
        ));
    }

    #[test]
    fn without_a_launcher_run_terminal_starts_nothing() {
        let tokio = tokio();
        let mut f = fixture(tokio.handle().clone(), 4);
        f.runtime.exec(run_terminal(1));
        assert_eq!(f.runtime.running(), 0);
        assert!(f.msgs.try_recv().is_err());
    }
}
