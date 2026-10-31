//! Converts table rows to domain types and back, and runs the SQL of each
//! operation.

use rusqlite::{Transaction, params};

use crate::bus::Persist;
use crate::store::error::StoreError;

/// Applies a fire-and-forget write.
pub(crate) fn persist(tx: &Transaction<'_>, persist: &Persist) -> Result<(), StoreError> {
    match persist {
        Persist::UiState { key, value } => {
            tx.execute(
                "INSERT INTO ui_state(key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
        }
    }
    Ok(())
}
