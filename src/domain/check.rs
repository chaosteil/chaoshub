//! CI checks of a pull request.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One check run or status context of a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Check {
    pub name: String,
    pub kind: CheckKind,
    pub status: CheckStatus,
    /// `None` until the check completes.
    pub conclusion: Option<CheckConclusion>,
    pub started: Option<DateTime<Utc>>,
    pub completed: Option<DateTime<Utc>>,
    pub url: Option<String>,
    /// The workflow run id, for re-runs of failed jobs.
    pub run_id: Option<u64>,
    /// The check suite id, for a re-request of the suite.
    pub suite_id: Option<u64>,
}

/// The GitHub source of a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum CheckKind {
    CheckRun,
    StatusContext,
}

/// The progress of a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum CheckStatus {
    Requested,
    Queued,
    Waiting,
    Pending,
    InProgress,
    Completed,
}

/// The result of a completed check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum CheckConclusion {
    Success,
    Failure,
    Neutral,
    Cancelled,
    Skipped,
    TimedOut,
    ActionRequired,
    StartupFailure,
    Stale,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{assert_round_trip, test_time};

    #[test]
    fn check_round_trip() {
        assert_round_trip(&Check {
            name: "build".into(),
            kind: CheckKind::CheckRun,
            status: CheckStatus::Completed,
            conclusion: Some(CheckConclusion::Failure),
            started: Some(test_time()),
            completed: Some(test_time()),
            url: Some("https://github.com/o/r/actions/runs/1".into()),
            run_id: Some(1),
            suite_id: Some(2),
        });
    }
}
