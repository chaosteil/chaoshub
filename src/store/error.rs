//! Errors of the store adapter.

/// An error of the SQLite cache.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// The store thread stopped.
    #[error("the store thread stopped")]
    Closed,
}
