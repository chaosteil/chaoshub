//! Adapter for the SQLite cache. The connection lives on its own thread.
//!
//! The cache is `~/.cache/chaoshub/chaoshub.db`. `PRAGMA user_version`
//! holds the schema version. On a mismatch the store drops every table
//! except `ui_state` and rebuilds, because the data is a cache.
#![allow(dead_code, reason = "the workers use the store in a later phase")]

pub(crate) mod error;
pub(crate) mod handle;
mod rows;
mod thread;

use std::path::Path;

use rusqlite::Connection;
use tokio::sync::mpsc;

use crate::store::error::StoreError;
use crate::store::handle::{CHANNEL_CAPACITY, StoreHandle};

/// The schema version. It must equal the `user_version` in `schema.sql`.
const SCHEMA_VERSION: i64 = 1;

/// The tables, the index and the `user_version`.
const SCHEMA: &str = include_str!("schema.sql");

/// The table that survives a schema rebuild.
const KEPT_TABLE: &str = "ui_state";

/// The SQLite cache.
pub(crate) struct Store;

/// The cached data that `Store::open` reads before the store thread
/// starts.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Snapshot {}

impl Store {
    /// Opens the cache at `path`, reads the snapshot directly, then moves
    /// the connection to the store thread.
    pub(crate) fn open(path: &Path) -> Result<(Snapshot, StoreHandle), StoreError> {
        let conn = open_connection(path)?;
        let snapshot = Snapshot::default();
        let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);
        thread::spawn(conn, path.to_path_buf(), rx)?;
        Ok((snapshot, StoreHandle::new(tx)))
    }
}

/// Opens the database at `path` and brings the schema to
/// `SCHEMA_VERSION`. It creates the parent directory. The path
/// `:memory:` opens an in-memory database.
fn open_connection(path: &Path) -> Result<Connection, StoreError> {
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let mut conn = Connection::open(path)?;
    migrate(&mut conn)?;
    Ok(conn)
}

/// Brings the schema to `SCHEMA_VERSION`. On a mismatch it drops every
/// table except `ui_state`, then runs `schema.sql`, in one transaction.
fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version == SCHEMA_VERSION {
        return Ok(());
    }
    tracing::info!(
        from = version,
        to = SCHEMA_VERSION,
        "store schema mismatch, rebuilding the cache"
    );
    let tx = conn.transaction()?;
    let tables: Vec<String> = {
        let mut stmt = tx.prepare(
            "SELECT name FROM sqlite_master \
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name != ?1",
        )?;
        stmt.query_map([KEPT_TABLE], |row| row.get(0))?
            .collect::<Result<_, _>>()?
    };
    for table in tables {
        tx.execute_batch(&format!(
            "DROP TABLE IF EXISTS \"{}\";",
            table.replace('"', "\"\"")
        ))?;
    }
    tx.execute_batch(SCHEMA)?;
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Persist;

    fn version(conn: &Connection) -> i64 {
        conn.pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap()
    }

    fn tables(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master \
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    #[test]
    fn schema_file_sets_the_schema_version() {
        assert!(SCHEMA.contains(&format!("PRAGMA user_version = {SCHEMA_VERSION};")));
    }

    #[test]
    fn open_creates_every_table() {
        let conn = open_connection(Path::new(":memory:")).unwrap();
        assert_eq!(version(&conn), SCHEMA_VERSION);
        assert_eq!(
            tables(&conn),
            [
                "local_state",
                "pr_details",
                "pull_requests",
                "repos",
                "sync_state",
                "ui_state"
            ]
        );
    }

    #[test]
    fn open_of_a_current_schema_keeps_the_data() {
        let mut conn = open_connection(Path::new(":memory:")).unwrap();
        conn.execute("INSERT INTO repos(owner, name) VALUES ('o', 'r')", [])
            .unwrap();
        migrate(&mut conn).unwrap();
        assert_eq!(count(&conn, "repos"), 1);
    }

    #[test]
    fn mismatch_rebuilds_and_keeps_ui_state() {
        let mut conn = open_connection(Path::new(":memory:")).unwrap();
        conn.execute_batch(
            "INSERT INTO repos(owner, name) VALUES ('o', 'r');
             INSERT INTO ui_state(key, value) VALUES ('filter', 'abc');
             CREATE TABLE old_table(x);
             PRAGMA user_version = 99;",
        )
        .unwrap();
        migrate(&mut conn).unwrap();
        assert_eq!(version(&conn), SCHEMA_VERSION);
        assert_eq!(count(&conn, "repos"), 0);
        assert_eq!(count(&conn, "ui_state"), 1);
        assert!(!tables(&conn).contains(&"old_table".to_owned()));
    }

    #[test]
    fn open_creates_the_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache/chaoshub.db");
        open_connection(&path).unwrap();
        assert!(path.exists());
    }

    fn ui_value(path: &Path, key: &str) -> Option<String> {
        let conn = Connection::open(path).unwrap();
        conn.query_row("SELECT value FROM ui_state WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .ok()
    }

    #[tokio::test]
    async fn persist_writes_ui_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chaoshub.db");
        let (_, handle) = Store::open(&path).unwrap();
        for value in ["a", "b"] {
            handle.try_persist(Persist::UiState {
                key: "mine.filter".into(),
                value: value.into(),
            });
        }
        handle.exec("SELECT 1").await.unwrap();
        assert_eq!(ui_value(&path, "mine.filter").as_deref(), Some("b"));
    }

    #[tokio::test]
    async fn sqlite_error_reopens_the_connection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chaoshub.db");
        let (_, handle) = Store::open(&path).unwrap();
        handle.try_persist(Persist::UiState {
            key: "k".into(),
            value: "v".into(),
        });
        handle.exec("CREATE TEMP TABLE marker(x)").await.unwrap();
        handle.exec("SELECT * FROM marker").await.unwrap();
        let err = handle.exec("SELECT * FROM no_such_table").await;
        assert!(matches!(err, Err(StoreError::Sqlite(_))));
        // A temp table lives only on its connection, so a reopen drops it.
        let err = handle.exec("SELECT * FROM marker").await;
        assert!(matches!(err, Err(StoreError::Sqlite(_))));
        handle.exec("SELECT 1").await.unwrap();
        assert_eq!(ui_value(&path, "k").as_deref(), Some("v"));
    }

    #[tokio::test]
    async fn store_thread_works_on_memory() {
        let (snapshot, handle) = Store::open(Path::new(":memory:")).unwrap();
        assert_eq!(snapshot, Snapshot::default());
        handle.exec("SELECT count(*) FROM ui_state").await.unwrap();
    }
}
