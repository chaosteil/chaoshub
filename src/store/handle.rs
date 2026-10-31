//! The handle that sends operations to the store thread.

use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, oneshot};

use crate::bus::Persist;
use crate::store::error::StoreError;

/// The capacity of the operation channel.
pub(crate) const CHANNEL_CAPACITY: usize = 64;

/// The reply channel of one operation.
pub(crate) type Reply<T> = oneshot::Sender<Result<T, StoreError>>;

/// One operation for the store thread. The thread runs each operation in
/// its own transaction and sends the result on `reply`.
#[derive(Debug)]
pub(crate) enum StoreOp {
    /// A fire-and-forget write. It has no reply.
    Persist(Persist),
    /// Runs SQL on the connection. Tests use it to inject errors.
    #[cfg(test)]
    Exec { sql: String, reply: Reply<()> },
}

/// The handle of the store thread. Clones share one bounded channel. The
/// thread stops when the last clone drops.
#[derive(Debug, Clone)]
pub(crate) struct StoreHandle {
    tx: mpsc::Sender<StoreOp>,
}

impl StoreHandle {
    pub(crate) fn new(tx: mpsc::Sender<StoreOp>) -> Self {
        Self { tx }
    }

    /// Sends a write without a wait. A full channel drops the write and
    /// logs it, because these writes are a convenience.
    pub(crate) fn try_persist(&self, persist: Persist) {
        match self.tx.try_send(StoreOp::Persist(persist)) {
            Ok(()) => {}
            Err(TrySendError::Full(op)) => {
                tracing::warn!(?op, "store channel full, write dropped");
            }
            Err(TrySendError::Closed(op)) => {
                tracing::warn!(?op, "store thread stopped, write dropped");
            }
        }
    }

    /// Sends the operation that `make` builds and waits for its reply.
    async fn request<T>(&self, make: impl FnOnce(Reply<T>) -> StoreOp) -> Result<T, StoreError> {
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(make(reply))
            .await
            .map_err(|_| StoreError::Closed)?;
        rx.await.map_err(|_| StoreError::Closed)?
    }

    /// Runs `sql` on the store thread.
    #[cfg(test)]
    pub(crate) async fn exec(&self, sql: &str) -> Result<(), StoreError> {
        let sql = sql.to_owned();
        self.request(|reply| StoreOp::Exec { sql, reply }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ui(key: &str) -> Persist {
        Persist::UiState {
            key: key.into(),
            value: "v".into(),
        }
    }

    #[test]
    fn try_persist_drops_a_write_on_a_full_channel() {
        let (tx, mut rx) = mpsc::channel(1);
        let handle = StoreHandle::new(tx);
        handle.try_persist(ui("a"));
        handle.try_persist(ui("b"));
        assert!(matches!(rx.try_recv(), Ok(StoreOp::Persist(p)) if p == ui("a")));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn request_to_a_stopped_thread_is_closed() {
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let handle = StoreHandle::new(tx);
        handle.try_persist(ui("a"));
        assert!(matches!(
            handle.exec("SELECT 1").await,
            Err(StoreError::Closed)
        ));
    }
}
