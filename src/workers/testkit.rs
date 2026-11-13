//! Test helpers: a worker context with in-memory adapters and the other
//! ends of its channels.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Mutex, mpsc, watch};

use crate::bus::{DetailReq, Interest, JjReq, Msg, PrRef, Session, SyncReq};
use crate::config::Config;
use crate::domain::keys::{PrKey, RepoKey};
use crate::github::{GithubClient, TokenSource};
use crate::store::Store;
use crate::store::handle::StoreHandle;
use crate::vcs::lock::RepoLocks;
use crate::workers::ctx::{WorkerCtx, WorkerMode};

/// The longest wait for a message in a test.
const RECV_TIMEOUT: Duration = Duration::from_secs(10);

/// The ends of the context channels that a test holds.
pub(crate) struct Harness {
    pub rx: mpsc::Receiver<Msg>,
    pub interest: watch::Sender<Interest>,
    pub session: watch::Sender<Session>,
    pub sync: mpsc::Sender<SyncReq>,
    pub detail: mpsc::Sender<DetailReq>,
    pub jj: mpsc::Sender<JjReq>,
}

impl Harness {
    /// The next message. It panics after `RECV_TIMEOUT`.
    pub(crate) async fn recv(&mut self) -> Msg {
        tokio::time::timeout(RECV_TIMEOUT, self.rx.recv())
            .await
            .unwrap_or_else(|_| panic!("no message in time"))
            .unwrap_or_else(|| panic!("channel closed"))
    }

    /// True when no message comes within `wait`. A closed channel is
    /// quiet.
    pub(crate) async fn quiet_for(&mut self, wait: Duration) -> bool {
        !matches!(
            tokio::time::timeout(wait, self.rx.recv()).await,
            Ok(Some(_))
        )
    }
}

/// A context with the default configuration and an in-memory store, for
/// a GitHub API at `base_url`.
pub(crate) fn ctx(base_url: &str) -> (WorkerCtx, Harness) {
    ctx_with(base_url, Config::default(), memory_store())
}

/// A context with `config` and `store`, for a GitHub API at `base_url`.
pub(crate) fn ctx_with(base_url: &str, config: Config, store: StoreHandle) -> (WorkerCtx, Harness) {
    let client = GithubClient::new(TokenSource::command("echo", &["tok"]), base_url).unwrap();
    let (tx, rx) = mpsc::channel(64);
    let (interest_tx, interest_rx) = watch::channel(interest(None, Vec::new()));
    let (session_tx, session_rx) = watch::channel(Session {
        login: None,
        repos: Vec::new(),
    });
    let (sync_tx, sync_rx) = mpsc::channel(16);
    let (detail_tx, detail_rx) = mpsc::channel(16);
    let (jj_tx, jj_rx) = mpsc::channel(16);
    let ctx = WorkerCtx {
        client: Arc::new(client),
        store,
        config: Arc::new(config),
        tx,
        interest: interest_rx,
        session: session_rx,
        sync_rx: Arc::new(Mutex::new(sync_rx)),
        detail_rx: Arc::new(Mutex::new(detail_rx)),
        jj_rx: Arc::new(Mutex::new(jj_rx)),
        locks: RepoLocks::default(),
        mode: WorkerMode::Lists,
    };
    let harness = Harness {
        rx,
        interest: interest_tx,
        session: session_tx,
        sync: sync_tx,
        detail: detail_tx,
        jj: jj_tx,
    };
    (ctx, harness)
}

/// A store on an in-memory database.
pub(crate) fn memory_store() -> StoreHandle {
    Store::open(Path::new(":memory:")).unwrap().1
}

/// The interest of the mine tab with `selected` and `visible`.
pub(crate) fn interest(selected: Option<PrRef>, visible: Vec<PrRef>) -> Interest {
    Interest {
        tab: "mine",
        scope: None,
        wants_local: false,
        selected,
        visible,
        show_hidden: false,
        ci_watch: Vec::new(),
    }
}

/// The repo `o/r`.
pub(crate) fn repo() -> RepoKey {
    RepoKey {
        owner: "o".into(),
        name: "r".into(),
    }
}

/// A reference to pull request `number` of `o/r`.
pub(crate) fn pr_ref(number: u64, node_id: &str, head_oid: &str) -> PrRef {
    PrRef {
        key: PrKey {
            repo: repo(),
            number,
        },
        node_id: node_id.to_owned(),
        head_oid: head_oid.to_owned(),
        base_ref: "main".to_owned(),
        base_oid: String::new(),
    }
}
