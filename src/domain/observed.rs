//! A value with the time it was observed, and the freshness rule.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A value and the time when it was observed at its source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Observed<T> {
    pub value: T,
    pub observed_at: DateTime<Utc>,
}
