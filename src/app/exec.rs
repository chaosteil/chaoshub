//! The runtime that executes commands.
//!
//! Each `Cmd` has one executor. This runtime sends the interest and the
//! session to the workers through watch channels, sends sync and jj
//! requests through their request channels, writes `ui_state` through the
//! store handle, and runs the clipboard and the browser on blocking
//! threads. The action commands come in phase E; it logs them.

use tokio::runtime::Handle;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

use crate::bus::{Cmd, Interest, JjReq, Session, SyncReq};
use crate::store::handle::StoreHandle;

/// The channels to the workers and the store.
#[derive(Debug)]
pub(crate) struct Senders {
    pub interest: watch::Sender<Interest>,
    pub session: watch::Sender<Session>,
    pub sync: mpsc::Sender<SyncReq>,
    pub jj: mpsc::Sender<JjReq>,
    pub store: StoreHandle,
}

/// Executes the commands of `update`.
#[derive(Debug)]
pub(crate) struct Runtime {
    /// The tokio runtime of the workers and the blocking tasks.
    handle: Handle,
    /// Cancels every task on quit.
    root: CancellationToken,
    senders: Senders,
    /// True after `Cmd::Quit`.
    quit: bool,
}

impl Runtime {
    /// Creates the runtime on the tokio runtime of `handle`.
    pub(crate) fn new(handle: Handle, senders: Senders) -> Self {
        Self {
            handle,
            root: CancellationToken::new(),
            senders,
            quit: false,
        }
    }

    /// The root token. Quit cancels it.
    #[allow(
        dead_code,
        reason = "the action tasks of phase E take child tokens of it"
    )]
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
            Cmd::Quit => {
                self.root.cancel();
                self.quit = true;
            }
            other @ (Cmd::RunAction { .. }
            | Cmd::RunTerminal { .. }
            | Cmd::FocusPane { .. }
            | Cmd::CancelAction { .. }) => {
                tracing::debug!(?other, "command not executed yet");
            }
        }
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
        Fixture {
            runtime: Runtime::new(handle, senders),
            interest,
            session,
            sync,
            jj,
            store,
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
}
