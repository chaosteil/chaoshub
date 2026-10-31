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

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use tokio::sync::mpsc;

use crate::bus::Scope;
use crate::domain::keys::RepoKey;
use crate::domain::local::LocalState;
use crate::domain::observed::Observed;
use crate::domain::pr::PullRequest;
use crate::domain::repo::RepoSettings;
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
pub(crate) struct Snapshot {
    /// The open pull requests and the scope of their last sync.
    pub prs: Vec<(Scope, PullRequest)>,
    /// The merge settings of every cached repo.
    pub repos: Vec<(RepoKey, RepoSettings)>,
    /// The newest sync time of the open pull requests: the age of the
    /// cache. `None` when the cache holds none.
    pub synced_at: Option<DateTime<Utc>>,
    /// The last jj scan of each repo.
    pub local: Vec<(RepoKey, Observed<LocalState>)>,
    /// The `ui_state` rows that `Cmd::Persist` wrote.
    pub ui_state: BTreeMap<String, String>,
}

/// The sync progress of one scope and one repo set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[allow(
    clippy::struct_field_names,
    reason = "the fields follow the columns of the sync_state table"
)]
pub(crate) struct SyncState {
    pub last_full: Option<DateTime<Utc>>,
    pub last_delta: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

impl Store {
    /// Opens the cache at `path`, reads the snapshot directly, then moves
    /// the connection to the store thread.
    pub(crate) fn open(path: &Path) -> Result<(Snapshot, StoreHandle), StoreError> {
        let conn = open_connection(path)?;
        let snapshot = rows::load_snapshot(&conn)?;
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
    use chrono::TimeDelta;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::bus::Persist;
    use crate::domain::detail::{Comment, PrDetail};
    use crate::domain::keys::PrKey;
    use crate::domain::local::Workspace;
    use crate::domain::pr::{CiState, MergeStateStatus, Mergeable, PrHead, PrState};
    use crate::domain::test_time;

    /// A pull request of `o/r` observed at `observed_at`.
    pub(crate) fn sample_pr(
        number: u64,
        state: PrState,
        observed_at: DateTime<Utc>,
    ) -> PullRequest {
        let observed = |value| Observed { value, observed_at };
        PullRequest {
            key: PrKey {
                repo: RepoKey {
                    owner: "o".into(),
                    name: "r".into(),
                },
                number,
            },
            node_id: format!("PR_{number}"),
            head: Observed {
                value: PrHead {
                    title: format!("Change {number}"),
                    body: "Body".into(),
                    author: "chaosteil".into(),
                    state,
                    is_draft: false,
                    head_ref: format!("chaosteil/change{number}"),
                    head_oid: format!("oid{number}"),
                    base_ref: "main".into(),
                    review_decision: None,
                    review_requested: false,
                    reviewers: Vec::new(),
                    stack: None,
                    auto_merge: false,
                    in_merge_queue: false,
                    mergeable: Mergeable::Unknown,
                    merge_state: MergeStateStatus::Unknown,
                    url: format!("https://github.com/o/r/pull/{number}"),
                    updated_at: observed_at,
                    repo_settings: RepoSettings {
                        squash: true,
                        merge: false,
                        rebase: true,
                        default_branch: None,
                    },
                },
                observed_at,
            },
            ci_state: observed(CiState::Success),
            checks: Observed {
                value: Vec::new(),
                observed_at,
            },
        }
    }

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

    fn reopen(path: &Path) -> Snapshot {
        Store::open(path).unwrap().0
    }

    #[tokio::test]
    async fn put_prs_shows_open_prs_in_the_next_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chaoshub.db");
        let (_, handle) = Store::open(&path).unwrap();
        let t = test_time();
        let open = sample_pr(1, PrState::Open, t);
        let merged = sample_pr(2, PrState::Merged, t);
        let team = sample_pr(3, PrState::Open, t + TimeDelta::seconds(5));
        handle
            .put_prs(Scope::Mine, vec![open.clone(), merged])
            .await
            .unwrap();
        handle
            .put_prs(Scope::Team, vec![team.clone()])
            .await
            .unwrap();

        let snapshot = reopen(&path);
        assert_eq!(snapshot.prs, [(Scope::Mine, open), (Scope::Team, team)]);
        assert_eq!(snapshot.synced_at, Some(t + TimeDelta::seconds(5)));
        assert_eq!(
            snapshot.repos,
            [(
                RepoKey {
                    owner: "o".into(),
                    name: "r".into()
                },
                RepoSettings {
                    squash: true,
                    merge: false,
                    rebase: true,
                    default_branch: None,
                }
            )]
        );
    }

    #[tokio::test]
    async fn put_prs_replaces_a_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chaoshub.db");
        let (_, handle) = Store::open(&path).unwrap();
        let t = test_time();
        handle
            .put_prs(Scope::Mine, vec![sample_pr(1, PrState::Open, t)])
            .await
            .unwrap();
        handle
            .put_prs(Scope::Mine, vec![sample_pr(1, PrState::Closed, t)])
            .await
            .unwrap();
        assert_eq!(reopen(&path).prs, []);
    }

    #[tokio::test]
    async fn drop_gone_keeps_prs_synced_after_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chaoshub.db");
        let (_, handle) = Store::open(&path).unwrap();
        let started = test_time();
        let old = sample_pr(1, PrState::Open, started - TimeDelta::seconds(60));
        let fresh = sample_pr(2, PrState::Open, started + TimeDelta::seconds(1));
        handle
            .put_prs(Scope::Mine, vec![old.clone(), fresh.clone()])
            .await
            .unwrap();
        let deleted = handle
            .drop_gone(vec![old.key.clone(), fresh.key.clone()], started)
            .await
            .unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(reopen(&path).prs, [(Scope::Mine, fresh)]);
    }

    #[test]
    fn sweep_deletes_closed_prs_after_30_days() {
        let conn = open_connection(Path::new(":memory:")).unwrap();
        let now = test_time();
        let prs = [
            sample_pr(1, PrState::Merged, now - TimeDelta::days(31)),
            sample_pr(2, PrState::Closed, now - TimeDelta::days(29)),
            sample_pr(3, PrState::Open, now - TimeDelta::days(90)),
        ];
        rows::put_prs(&conn, Scope::Mine, &prs).unwrap();
        assert_eq!(rows::sweep_closed(&conn, now).unwrap(), 1);
        let left: i64 = conn
            .query_row("SELECT count(*) FROM pull_requests", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 2);
    }

    #[test]
    fn snapshot_skips_a_row_that_does_not_parse() {
        let conn = open_connection(Path::new(":memory:")).unwrap();
        let t = test_time();
        rows::put_prs(&conn, Scope::Mine, &[sample_pr(1, PrState::Open, t)]).unwrap();
        conn.execute(
            "INSERT INTO pull_requests(owner, name, number, scope, state, synced_at, json) \
             VALUES ('o', 'r', 2, 'mine', 'OPEN', ?1, x'00')",
            [t],
        )
        .unwrap();
        let snapshot = rows::load_snapshot(&conn).unwrap();
        assert_eq!(snapshot.prs.len(), 1);
    }

    #[tokio::test]
    async fn sync_state_round_trip() {
        let (_, handle) = Store::open(Path::new(":memory:")).unwrap();
        assert_eq!(
            handle.sync_state(Scope::Mine, "set1".into()).await.unwrap(),
            None
        );
        let state = SyncState {
            last_full: Some(test_time()),
            last_delta: Some(test_time() + TimeDelta::milliseconds(1500)),
            last_error: Some("offline".into()),
        };
        handle
            .put_sync_state(Scope::Mine, "set1".into(), state.clone())
            .await
            .unwrap();
        assert_eq!(
            handle.sync_state(Scope::Mine, "set1".into()).await.unwrap(),
            Some(state)
        );
        assert_eq!(
            handle.sync_state(Scope::Team, "set1".into()).await.unwrap(),
            None
        );
    }

    fn key(number: u64) -> PrKey {
        sample_pr(number, PrState::Open, test_time()).key
    }

    #[tokio::test]
    async fn detail_round_trip() {
        let (_, handle) = Store::open(Path::new(":memory:")).unwrap();
        assert_eq!(handle.detail(key(1)).await.unwrap(), None);
        let detail = Observed {
            value: PrDetail {
                comments: vec![Comment {
                    author: "colleague".into(),
                    body: "Looks good.".into(),
                    created_at: test_time(),
                    url: "https://github.com/o/r/pull/1#issuecomment-1".into(),
                }],
                reviews: Vec::new(),
                threads: Vec::new(),
                head_oid: "oid1".into(),
            },
            observed_at: test_time(),
        };
        handle.put_detail(key(1), detail.clone()).await.unwrap();
        assert_eq!(handle.detail(key(1)).await.unwrap(), Some(detail.clone()));
        assert_eq!(handle.detail(key(2)).await.unwrap(), None);

        let newer = Observed {
            value: PrDetail {
                head_oid: "oid2".into(),
                ..detail.value
            },
            observed_at: test_time() + TimeDelta::seconds(1),
        };
        handle.put_detail(key(1), newer.clone()).await.unwrap();
        assert_eq!(handle.detail(key(1)).await.unwrap(), Some(newer));
    }

    #[tokio::test]
    async fn local_state_and_ui_state_load_into_the_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chaoshub.db");
        let (snapshot, handle) = Store::open(&path).unwrap();
        assert_eq!(snapshot.local, []);
        assert_eq!(snapshot.ui_state, BTreeMap::new());

        let repo = key(1).repo;
        let scan = |change_id: &str, offset| Observed {
            value: LocalState {
                workspaces: vec![Workspace {
                    name: "default".into(),
                    change_id: change_id.into(),
                    commit_id: "abc123".into(),
                }],
                changes: Vec::new(),
            },
            observed_at: test_time() + TimeDelta::seconds(offset),
        };
        handle
            .put_local(repo.clone(), scan("kxyz", 0))
            .await
            .unwrap();
        handle
            .put_local(repo.clone(), scan("kzzz", 1))
            .await
            .unwrap();
        handle.try_persist(Persist::UiState {
            key: "team.filter".into(),
            value: "deploy".into(),
        });
        handle.exec("SELECT 1").await.unwrap();

        let snapshot = reopen(&path);
        assert_eq!(snapshot.local, [(repo, scan("kzzz", 1))]);
        assert_eq!(
            snapshot.ui_state,
            BTreeMap::from([("team.filter".to_owned(), "deploy".to_owned())])
        );
    }

    /// The startup budget is 30 ms for 500 pull requests. The test allows
    /// five times that, so a slow machine does not fail it.
    #[test]
    fn startup_load_of_500_prs_is_fast() {
        use std::time::{Duration, Instant};

        use crate::domain::check::{Check, CheckConclusion, CheckKind, CheckStatus};

        let conn = open_connection(Path::new(":memory:")).unwrap();
        let t = test_time();
        let prs: Vec<PullRequest> = (1..=500)
            .map(|number| {
                let mut pr = sample_pr(number, PrState::Open, t);
                pr.head.value.body = "A pull request body with a few lines.\n".repeat(20);
                pr.checks.value = (0..10)
                    .map(|i| Check {
                        name: format!("check {i}"),
                        kind: CheckKind::CheckRun,
                        status: CheckStatus::Completed,
                        conclusion: Some(CheckConclusion::Success),
                        started: Some(t),
                        completed: Some(t),
                        url: Some(format!("https://github.com/o/r/actions/runs/{i}")),
                        run_id: Some(i),
                        suite_id: Some(i),
                    })
                    .collect();
                pr
            })
            .collect();
        let (mine, team) = prs.split_at(250);
        rows::put_prs(&conn, Scope::Mine, mine).unwrap();
        rows::put_prs(&conn, Scope::Team, team).unwrap();

        let start = Instant::now();
        let snapshot = rows::load_snapshot(&conn).unwrap();
        let elapsed = start.elapsed();

        assert_eq!(snapshot.prs.len(), 500);
        assert!(
            elapsed < Duration::from_millis(150),
            "startup load took {elapsed:?}"
        );
    }
}
