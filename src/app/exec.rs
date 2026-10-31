//! The runtime that executes commands.
//!
//! Each `Cmd` has one executor. This runtime sends the interest and the
//! session to the workers through watch channels, sends sync and jj
//! requests through their request channels, writes `ui_state` through the
//! store handle, and runs the clipboard and the browser on blocking
//! threads. It runs the action tasks in a `JoinSet`, each with a child
//! token of the root token, and sends `ActionEvent::Done` with the result
//! of each task.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::Handle;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::actions::{Action, Services};
use crate::bus::{
    ActionEvent, ActionInput, Cmd, Interest, JjReq, Msg, Selection, Session, SyncReq, TargetKey,
};
use crate::github::GithubClient;
use crate::store::handle::StoreHandle;
use crate::vcs::lock::RepoLocks;

/// The longest wait on quit for the cancelled tasks to end.
const QUIT_WAIT: Duration = Duration::from_millis(500);

/// The key of a running task: the action id and the target.
type TaskKey = (&'static str, TargetKey);

/// The channels to the workers and the store.
#[derive(Debug)]
pub(crate) struct Senders {
    pub interest: watch::Sender<Interest>,
    pub session: watch::Sender<Session>,
    pub sync: mpsc::Sender<SyncReq>,
    pub jj: mpsc::Sender<JjReq>,
    pub store: StoreHandle,
}

/// What the action tasks need: the registry and the parts of
/// `Services`.
pub(crate) struct ActionEnv {
    /// The actions that `Cmd::RunAction` names by id.
    pub registry: &'static [Box<dyn Action>],
    pub client: Arc<GithubClient>,
    pub locks: RepoLocks,
    /// Sends `Msg::ActionEvent` to `app`.
    pub tx: mpsc::Sender<Msg>,
}

impl std::fmt::Debug for ActionEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionEnv")
            .field("actions", &self.registry.len())
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
    /// True after `Cmd::Quit`.
    quit: bool,
}

impl Runtime {
    /// Creates the runtime on the tokio runtime of `handle`.
    pub(crate) fn new(handle: Handle, senders: Senders, env: ActionEnv) -> Self {
        Self {
            handle,
            root: CancellationToken::new(),
            senders,
            env,
            tasks: JoinSet::new(),
            tokens: HashMap::new(),
            next: 0,
            quit: false,
        }
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
            Cmd::ScanJj(repo) => self.request(&self.senders.jj, JjReq::Scan(repo)),
            Cmd::Persist(persist) => self.senders.store.try_persist(persist),
            Cmd::Copy(text) => {
                self.handle.spawn_blocking(move || copy(&text));
            }
            Cmd::OpenUrl(url) => {
                self.handle.spawn_blocking(move || {
                    if let Err(err) = open::that(&url) {
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
            other @ (Cmd::RunTerminal { .. } | Cmd::FocusPane { .. }) => {
                tracing::debug!(?other, "command not executed yet");
            }
        }
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

/// Puts `text` on the system clipboard.
fn copy(text: &str) {
    let result = arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text));
    if let Err(err) = result {
        tracing::warn!(%err, "cannot copy to the clipboard");
    }
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
        jj: mpsc::Receiver<JjReq>,
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
        }
    }

    fn fixture(handle: Handle, capacity: usize) -> Fixture {
        fixture_with(handle, capacity, Vec::new())
    }

    /// The runtime with the actions of `registry`.
    fn fixture_with(handle: Handle, capacity: usize, registry: Vec<Box<dyn Action>>) -> Fixture {
        let (interest_tx, interest) = watch::channel(interest("none"));
        let (session_tx, session) = watch::channel(Session {
            login: None,
            repos: Vec::new(),
        });
        let (sync_tx, sync) = mpsc::channel(capacity);
        let (jj_tx, jj) = mpsc::channel(capacity);
        let (store_tx, store) = mpsc::channel(4);
        let senders = Senders {
            interest: interest_tx,
            session: session_tx,
            sync: sync_tx,
            jj: jj_tx,
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
        };
        Fixture {
            runtime: Runtime::new(handle, senders, env),
            interest,
            session,
            sync,
            jj,
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
        f.runtime.exec(Cmd::ScanJj(repo()));
        assert_eq!(f.sync.recv().await, Some(SyncReq::Full(Scope::Team)));
        assert_eq!(f.sync.recv().await, Some(SyncReq::Refresh(Vec::new())));
        assert_eq!(f.jj.recv().await, Some(JjReq::Scan(repo())));
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
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while f.runtime.running() != 1 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(f.runtime.running(), 1);
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
}
