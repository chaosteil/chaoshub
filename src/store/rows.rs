//! Converts table rows to domain types and back, and runs the SQL of each
//! operation.

use chrono::{DateTime, TimeDelta, Utc};
use rusqlite::{Connection, OptionalExtension, params};

use crate::bus::{Persist, Scope};
use crate::domain::detail::PrDetail;
use crate::domain::files::PrFiles;
use crate::domain::keys::{PrKey, RepoKey};
use crate::domain::local::LocalState;
use crate::domain::observed::Observed;
use crate::domain::pr::{PrState, PullRequest};
use crate::domain::repo::RepoSettings;
use crate::store::error::StoreError;
use crate::store::{PrVersion, Snapshot, SyncState};

/// Closed and merged pull requests stay this long after their last sync.
const SWEEP_AFTER_DAYS: i64 = 30;

/// Applies a fire-and-forget write.
pub(crate) fn persist(conn: &Connection, persist: &Persist) -> Result<(), StoreError> {
    match persist {
        Persist::UiState { key, value } => {
            conn.execute(
                "INSERT INTO ui_state(key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
        }
    }
    Ok(())
}

/// A pull request number as a SQLite integer.
fn number(number: u64) -> Result<i64, StoreError> {
    i64::try_from(number)
        .map_err(|err| rusqlite::Error::ToSqlConversionFailure(Box::new(err)).into())
}

/// The text of a scope in the `scope` columns.
fn scope_text(scope: Scope) -> &'static str {
    match scope {
        Scope::Mine => "mine",
        Scope::Team => "team",
    }
}

fn scope_from_text(text: &str) -> Option<Scope> {
    match text {
        "mine" => Some(Scope::Mine),
        "team" => Some(Scope::Team),
        _ => None,
    }
}

/// The text of a state in the `state` column.
fn state_text(state: PrState) -> &'static str {
    match state {
        PrState::Open => "OPEN",
        PrState::Closed => "CLOSED",
        PrState::Merged => "MERGED",
    }
}

/// Writes the pull requests of one scope and the merge settings of their
/// repos. `synced_at` is the time when the head was observed.
pub(crate) fn put_prs(
    conn: &Connection,
    scope: Scope,
    prs: &[PullRequest],
) -> Result<(), StoreError> {
    let mut put_pr = conn.prepare_cached(
        "INSERT INTO pull_requests(owner, name, number, scope, state, updated_at, synced_at, json) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT(owner, name, number) DO UPDATE SET \
           scope = excluded.scope, state = excluded.state, updated_at = excluded.updated_at, \
           synced_at = excluded.synced_at, json = excluded.json",
    )?;
    for pr in prs {
        let head = &pr.head.value;
        put_pr.execute(params![
            pr.key.repo.owner,
            pr.key.repo.name,
            number(pr.key.number)?,
            scope_text(scope),
            state_text(head.state),
            head.updated_at,
            pr.head.observed_at,
            serde_json::to_vec(pr)?,
        ])?;
        put_repo(conn, &pr.key.repo, &head.repo_settings)?;
    }
    Ok(())
}

/// Writes the merge settings of a repo. A missing default branch keeps the
/// stored one.
fn put_repo(conn: &Connection, key: &RepoKey, settings: &RepoSettings) -> Result<(), StoreError> {
    conn.prepare_cached(
        "INSERT INTO repos(owner, name, squash_allowed, merge_allowed, rebase_allowed, default_branch) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(owner, name) DO UPDATE SET \
           squash_allowed = excluded.squash_allowed, merge_allowed = excluded.merge_allowed, \
           rebase_allowed = excluded.rebase_allowed, \
           default_branch = coalesce(excluded.default_branch, repos.default_branch)",
    )?
    .execute(params![
        key.owner,
        key.name,
        settings.squash,
        settings.merge,
        settings.rebase,
        settings.default_branch,
    ])?;
    Ok(())
}

/// Deletes the gone candidates of a full sync that were not synced after
/// `started_at`. Returns the number of deleted rows.
pub(crate) fn drop_gone(
    conn: &Connection,
    keys: &[PrKey],
    started_at: DateTime<Utc>,
) -> Result<usize, StoreError> {
    let mut delete = conn.prepare_cached(
        "DELETE FROM pull_requests WHERE owner = ?1 AND name = ?2 AND number = ?3 \
         AND julianday(synced_at) <= julianday(?4)",
    )?;
    let mut deleted = 0;
    for key in keys {
        deleted += delete.execute(params![
            key.repo.owner,
            key.repo.name,
            number(key.number)?,
            started_at
        ])?;
    }
    Ok(deleted)
}

/// Deletes the closed and merged pull requests that were last synced more
/// than 30 days before `now`. Returns the number of deleted rows.
pub(crate) fn sweep_closed(conn: &Connection, now: DateTime<Utc>) -> Result<usize, StoreError> {
    let cutoff = now - TimeDelta::days(SWEEP_AFTER_DAYS);
    Ok(conn.execute(
        "DELETE FROM pull_requests WHERE state != 'OPEN' \
         AND julianday(synced_at) < julianday(?1)",
        params![cutoff],
    )?)
}

/// The sync state of a scope and a repo set.
pub(crate) fn sync_state(
    conn: &Connection,
    scope: Scope,
    repo_set: &str,
) -> Result<Option<SyncState>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT last_full, last_delta, last_error FROM sync_state \
             WHERE scope = ?1 AND repo_set = ?2",
            params![scope_text(scope), repo_set],
            |row| {
                Ok(SyncState {
                    last_full: row.get(0)?,
                    last_delta: row.get(1)?,
                    last_error: row.get(2)?,
                })
            },
        )
        .optional()?)
}

/// Writes the sync state of a scope and a repo set.
pub(crate) fn put_sync_state(
    conn: &Connection,
    scope: Scope,
    repo_set: &str,
    state: &SyncState,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO sync_state(scope, repo_set, last_full, last_delta, last_error) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT(scope, repo_set) DO UPDATE SET \
           last_full = excluded.last_full, last_delta = excluded.last_delta, \
           last_error = excluded.last_error",
        params![
            scope_text(scope),
            repo_set,
            state.last_full,
            state.last_delta,
            state.last_error
        ],
    )?;
    Ok(())
}

/// The versions of the open pull requests of `scope`. A row that does not
/// parse is skipped and logged.
pub(crate) fn pr_versions(
    conn: &Connection,
    scope: Scope,
) -> Result<Vec<(PrKey, PrVersion)>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT json FROM pull_requests WHERE state = 'OPEN' AND scope = ?1 \
         ORDER BY owner, name, number",
    )?;
    let rows = stmt.query_map(params![scope_text(scope)], |row| row.get::<_, Vec<u8>>(0))?;
    let mut versions = Vec::new();
    for row in rows {
        match serde_json::from_slice::<PullRequest>(&row?) {
            Ok(pr) => versions.push((
                pr.key,
                PrVersion {
                    updated_at: pr.head.value.updated_at,
                    head_oid: pr.head.value.head_oid,
                    observed_at: pr.head.observed_at,
                },
            )),
            Err(err) => tracing::warn!(%err, "cached pull request does not parse, skipped"),
        }
    }
    Ok(versions)
}

/// Writes the conversation of a pull request. `fetched_at` is the time
/// when it was observed.
pub(crate) fn put_detail(
    conn: &Connection,
    key: &PrKey,
    detail: &Observed<PrDetail>,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO pr_details(owner, name, number, fetched_at, head_oid, json) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(owner, name, number) DO UPDATE SET \
           fetched_at = excluded.fetched_at, head_oid = excluded.head_oid, json = excluded.json",
        params![
            key.repo.owner,
            key.repo.name,
            number(key.number)?,
            detail.observed_at,
            detail.value.head_oid,
            serde_json::to_vec(&detail.value)?,
        ],
    )?;
    Ok(())
}

/// The cached conversation of a pull request. `None` when the cache has
/// none.
pub(crate) fn detail(
    conn: &Connection,
    key: &PrKey,
) -> Result<Option<Observed<PrDetail>>, StoreError> {
    let row = conn
        .query_row(
            "SELECT fetched_at, json FROM pr_details WHERE owner = ?1 AND name = ?2 AND number = ?3",
            params![key.repo.owner, key.repo.name, number(key.number)?],
            |row| Ok((row.get::<_, DateTime<Utc>>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .optional()?;
    let Some((observed_at, json)) = row else {
        return Ok(None);
    };
    Ok(Some(Observed {
        value: serde_json::from_slice(&json)?,
        observed_at,
    }))
}

/// Writes the changed files of a pull request. `fetched_at` is the time
/// when they were observed.
pub(crate) fn put_files(
    conn: &Connection,
    key: &PrKey,
    files: &Observed<PrFiles>,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO pr_files(owner, name, number, fetched_at, head_oid, json) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(owner, name, number) DO UPDATE SET \
           fetched_at = excluded.fetched_at, head_oid = excluded.head_oid, json = excluded.json",
        params![
            key.repo.owner,
            key.repo.name,
            number(key.number)?,
            files.observed_at,
            files.value.head_oid,
            serde_json::to_vec(&files.value)?,
        ],
    )?;
    Ok(())
}

/// The cached changed files of a pull request. `None` when the cache has
/// none.
pub(crate) fn files(
    conn: &Connection,
    key: &PrKey,
) -> Result<Option<Observed<PrFiles>>, StoreError> {
    let row = conn
        .query_row(
            "SELECT fetched_at, json FROM pr_files WHERE owner = ?1 AND name = ?2 AND number = ?3",
            params![key.repo.owner, key.repo.name, number(key.number)?],
            |row| Ok((row.get::<_, DateTime<Utc>>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .optional()?;
    let Some((observed_at, json)) = row else {
        return Ok(None);
    };
    Ok(Some(Observed {
        value: serde_json::from_slice(&json)?,
        observed_at,
    }))
}

/// Writes the result of one jj scan of a repo. `scanned_at` is the time
/// when it was observed.
pub(crate) fn put_local(
    conn: &Connection,
    repo: &RepoKey,
    state: &Observed<LocalState>,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO local_state(owner, name, scanned_at, json) VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT(owner, name) DO UPDATE SET \
           scanned_at = excluded.scanned_at, json = excluded.json",
        params![
            repo.owner,
            repo.name,
            state.observed_at,
            serde_json::to_vec(&state.value)?,
        ],
    )?;
    Ok(())
}

/// Reads the startup data: the open pull requests, the repo settings, the
/// jj scans and the ui state. A row that does not parse is skipped and
/// logged.
pub(crate) fn load_snapshot(conn: &Connection) -> Result<Snapshot, StoreError> {
    let mut snapshot = Snapshot::default();

    let mut stmt = conn.prepare(
        "SELECT scope, synced_at, json FROM pull_requests WHERE state = 'OPEN' \
         ORDER BY owner, name, number",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, DateTime<Utc>>(1)?,
            row.get::<_, Vec<u8>>(2)?,
        ))
    })?;
    for row in rows {
        let (scope, synced_at, json) = row?;
        let Some(scope) = scope_from_text(&scope) else {
            tracing::warn!(scope, "cached pull request with an unknown scope, skipped");
            continue;
        };
        match serde_json::from_slice::<PullRequest>(&json) {
            Ok(pr) => snapshot.prs.push((scope, pr)),
            Err(err) => {
                tracing::warn!(%err, "cached pull request does not parse, skipped");
                continue;
            }
        }
        snapshot.synced_at = snapshot.synced_at.max(Some(synced_at));
    }

    let mut stmt = conn.prepare(
        "SELECT owner, name, squash_allowed, merge_allowed, rebase_allowed, default_branch \
         FROM repos ORDER BY owner, name",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            RepoKey {
                owner: row.get(0)?,
                name: row.get(1)?,
            },
            RepoSettings {
                squash: row.get::<_, Option<bool>>(2)?.unwrap_or(false),
                merge: row.get::<_, Option<bool>>(3)?.unwrap_or(false),
                rebase: row.get::<_, Option<bool>>(4)?.unwrap_or(false),
                default_branch: row.get(5)?,
                squash_title_default: None,
                squash_message_default: None,
            },
        ))
    })?;
    for row in rows {
        snapshot.repos.push(row?);
    }

    let mut stmt =
        conn.prepare("SELECT owner, name, scanned_at, json FROM local_state ORDER BY owner, name")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            RepoKey {
                owner: row.get(0)?,
                name: row.get(1)?,
            },
            row.get::<_, DateTime<Utc>>(2)?,
            row.get::<_, Vec<u8>>(3)?,
        ))
    })?;
    for row in rows {
        let (repo, observed_at, json) = row?;
        match serde_json::from_slice::<LocalState>(&json) {
            Ok(value) => snapshot.local.push((repo, Observed { value, observed_at })),
            Err(err) => tracing::warn!(%err, ?repo, "cached jj scan does not parse, skipped"),
        }
    }

    let mut stmt = conn.prepare("SELECT key, value FROM ui_state")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    for row in rows {
        let (key, value) = row?;
        snapshot.ui_state.insert(key, value.unwrap_or_default());
    }
    Ok(snapshot)
}
