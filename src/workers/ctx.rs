//! The context that every worker gets: adapters, configuration and
//! channels.

use std::sync::Arc;

use tokio::sync::{Mutex, mpsc, watch};

use crate::bus::{Interest, JjReq, Msg, Session, SyncReq};
use crate::config::Config;
use crate::github::GithubClient;
use crate::store::handle::StoreHandle;
use crate::vcs::lock::RepoLocks;

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
    /// Requests to the jj worker.
    pub jj_rx: Arc<Mutex<mpsc::Receiver<JjReq>>>,
    pub locks: RepoLocks,
}
