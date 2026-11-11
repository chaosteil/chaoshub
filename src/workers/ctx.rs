//! The context that every worker gets: adapters, configuration and
//! channels.

use std::sync::Arc;

use tokio::sync::{Mutex, mpsc, watch};

use crate::bus::{DetailReq, Interest, JjReq, Msg, Session, SyncReq};
use crate::config::Config;
use crate::domain::keys::PrKey;
use crate::github::GithubClient;
use crate::store::handle::StoreHandle;
use crate::vcs::lock::RepoLocks;

/// What the process shows: the lists, or one pinned pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkerMode {
    /// The tabs with the lists of pull requests.
    Lists,
    /// One pull request, from `chaoshub pr OWNER/NAME NUMBER`.
    Pinned(PrKey),
}

/// The adapters, the configuration and the channels of the workers. A
/// restarted worker gets a clone, so the request receivers sit behind a
/// shared lock.
#[derive(Clone)]
pub(crate) struct WorkerCtx {
    pub client: Arc<GithubClient>,
    pub store: StoreHandle,
    pub config: Arc<Config>,
    /// Sends messages to `app`.
    pub tx: mpsc::Sender<Msg>,
    pub interest: watch::Receiver<Interest>,
    pub session: watch::Receiver<Session>,
    /// Requests to the sync worker.
    pub sync_rx: Arc<Mutex<mpsc::Receiver<SyncReq>>>,
    /// Requests to the detail worker.
    pub detail_rx: Arc<Mutex<mpsc::Receiver<DetailReq>>>,
    /// Requests to the jj worker.
    pub jj_rx: Arc<Mutex<mpsc::Receiver<JjReq>>>,
    pub locks: RepoLocks,
    /// The lists, or one pinned pull request.
    pub mode: WorkerMode,
}
