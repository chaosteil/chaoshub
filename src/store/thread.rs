//! The store thread loop: one transaction per operation.
//!
//! The thread owns the `Connection`. On a SQLite error it logs the error
//! and reopens the connection, so one bad state does not stop the cache.
//! It never panics by design: every error becomes a reply or a log line.

use std::path::PathBuf;

use chrono::Utc;
use rusqlite::{Connection, Transaction};
use tokio::sync::mpsc;

use crate::store::error::StoreError;
use crate::store::handle::StoreOp;
use crate::store::{open_connection, rows};

/// Starts the store thread. It runs until every `StoreHandle` drops.
pub(crate) fn spawn(
    conn: Connection,
    path: PathBuf,
    rx: mpsc::Receiver<StoreOp>,
) -> Result<(), StoreError> {
    std::thread::Builder::new()
        .name("chaoshub-store".to_owned())
        .spawn(move || run(conn, path, rx))?;
    Ok(())
}

fn run(conn: Connection, path: PathBuf, mut rx: mpsc::Receiver<StoreOp>) {
    let mut store = StoreThread {
        conn: Some(conn),
        path,
    };
    match store.transact(|tx| rows::sweep_closed(tx, Utc::now())) {
        Ok(swept) => tracing::debug!(swept, "swept closed pull requests"),
        Err(err) => tracing::warn!(%err, "sweep of closed pull requests failed"),
    }
    while let Some(op) = rx.blocking_recv() {
        store.handle(op);
    }
    tracing::debug!("store thread stopped");
}

/// The state of the store thread. `conn` is `None` after a reopen failed;
/// the next operation tries again.
struct StoreThread {
    conn: Option<Connection>,
    path: PathBuf,
}

impl StoreThread {
    fn handle(&mut self, op: StoreOp) {
        match op {
            StoreOp::Persist(persist) => {
                if let Err(err) = self.transact(|tx| rows::persist(tx, &persist)) {
                    tracing::warn!(%err, ?persist, "store write failed");
                }
            }
            StoreOp::PutPrs { scope, prs, reply } => {
                let _ = reply.send(self.transact(|tx| rows::put_prs(tx, scope, &prs)));
            }
            StoreOp::DropGone {
                keys,
                started_at,
                reply,
            } => {
                let _ = reply.send(self.transact(|tx| rows::drop_gone(tx, &keys, started_at)));
            }
            StoreOp::SyncState {
                scope,
                repo_set,
                reply,
            } => {
                let _ = reply.send(self.transact(|tx| rows::sync_state(tx, scope, &repo_set)));
            }
            StoreOp::PutSyncState {
                scope,
                repo_set,
                state,
                reply,
            } => {
                let _ = reply
                    .send(self.transact(|tx| rows::put_sync_state(tx, scope, &repo_set, &state)));
            }
            StoreOp::PutDetail { key, detail, reply } => {
                let _ = reply.send(self.transact(|tx| rows::put_detail(tx, &key, &detail)));
            }
            StoreOp::Detail { key, reply } => {
                let _ = reply.send(self.transact(|tx| rows::detail(tx, &key)));
            }
            StoreOp::PutFiles { key, files, reply } => {
                let _ = reply.send(self.transact(|tx| rows::put_files(tx, &key, &files)));
            }
            StoreOp::Files { key, reply } => {
                let _ = reply.send(self.transact(|tx| rows::files(tx, &key)));
            }
            StoreOp::PutLocal { repo, state, reply } => {
                let _ = reply.send(self.transact(|tx| rows::put_local(tx, &repo, &state)));
            }
            #[cfg(test)]
            StoreOp::Exec { sql, reply } => {
                let result = self.transact(|tx| Ok(tx.execute_batch(&sql)?));
                let _ = reply.send(result);
            }
        }
    }

    /// Runs `f` in one transaction. A SQLite error reopens the connection.
    fn transact<T>(
        &mut self,
        f: impl FnOnce(&Transaction<'_>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let result = match self.connection() {
            Ok(conn) => in_transaction(conn, f),
            Err(err) => Err(err),
        };
        if let Err(StoreError::Sqlite(err)) = &result {
            tracing::warn!(%err, "store error, reopening the connection");
            self.reopen();
        }
        result
    }

    /// The open connection. It opens one when the last reopen failed.
    fn connection(&mut self) -> Result<&mut Connection, StoreError> {
        let conn = match self.conn.take() {
            Some(conn) => conn,
            None => open_connection(&self.path)?,
        };
        Ok(self.conn.insert(conn))
    }

    fn reopen(&mut self) {
        self.conn = None;
        match open_connection(&self.path) {
            Ok(conn) => self.conn = Some(conn),
            Err(err) => tracing::warn!(%err, "store reopen failed, retrying on the next operation"),
        }
    }
}

fn in_transaction<T>(
    conn: &mut Connection,
    f: impl FnOnce(&Transaction<'_>) -> Result<T, StoreError>,
) -> Result<T, StoreError> {
    let tx = conn.transaction()?;
    let value = f(&tx)?;
    tx.commit()?;
    Ok(value)
}
