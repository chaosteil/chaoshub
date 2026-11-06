//! Pull requests and their states.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::check::Check;
use crate::domain::keys::PrKey;
use crate::domain::observed::Observed;
use crate::domain::repo::RepoSettings;
use crate::domain::stack::StackRef;

/// A pull request. Each field group has its own observation time, so a
/// newer value of one group never loses to an older one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PullRequest {
    pub key: PrKey,
    /// The GraphQL node id.
    pub node_id: String,
    pub head: Observed<PrHead>,
    pub ci_state: Observed<CiState>,
    pub checks: Observed<Vec<Check>>,
}

/// The fields of a pull request that the list sync reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PrHead {
    pub title: String,
    pub body: String,
    pub author: String,
    pub state: PrState,
    pub is_draft: bool,
    pub head_ref: String,
    pub head_oid: String,
    pub base_ref: String,
    pub additions: u32,
    pub deletions: u32,
    pub changed_files: u32,
    /// `None` when the repo requires no review decision.
    pub review_decision: Option<ReviewDecision>,
    /// True when the review of the user or of one of the user's teams is
    /// requested.
    pub review_requested: bool,
    /// The logins of the requested reviewers.
    pub reviewers: Vec<String>,
    /// The latest opinionated reviews that approve.
    #[serde(default)]
    pub approvals: u32,
    /// The latest opinionated reviews that request changes.
    #[serde(default)]
    pub changes_requested: u32,
    /// `None` when the pull request is not in a stack.
    pub stack: Option<StackRef>,
    pub auto_merge: bool,
    pub in_merge_queue: bool,
    pub mergeable: Mergeable,
    pub merge_state: MergeStateStatus,
    pub url: String,
    pub updated_at: DateTime<Utc>,
    pub repo_settings: RepoSettings,
}

/// The state of a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PrState {
    Open,
    Closed,
    Merged,
}

/// Whether a pull request merges without conflicts. GitHub computes it
/// lazily, so `Unknown` means "not known yet".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(
    clippy::enum_variant_names,
    reason = "the variants follow the GitHub values"
)]
pub(crate) enum Mergeable {
    Mergeable,
    Conflicting,
    Unknown,
}

/// The merge state of a pull request, as GitHub reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum MergeStateStatus {
    Behind,
    Blocked,
    Clean,
    Dirty,
    Draft,
    HasHooks,
    Unknown,
    Unstable,
}

/// The review decision of a pull request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ReviewDecision {
    Approved,
    ChangesRequested,
    ReviewRequired,
}

/// The combined CI state of the head commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum CiState {
    Success,
    Failure,
    Pending,
    /// The head commit has no checks.
    None,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::assert_round_trip;
    use crate::domain::check::{CheckKind, CheckStatus};
    use crate::domain::keys::RepoKey;
    use crate::domain::test_time;

    fn observed<T>(value: T) -> Observed<T> {
        Observed {
            value,
            observed_at: test_time(),
        }
    }

    /// A pull request with every field set.
    fn pull_request() -> PullRequest {
        PullRequest {
            key: PrKey {
                repo: RepoKey {
                    owner: "chaosteil".into(),
                    name: "chaoshub".into(),
                },
                number: 42,
            },
            node_id: "PR_1".into(),
            head: observed(PrHead {
                title: "Add the thing".into(),
                body: "Body".into(),
                author: "chaosteil".into(),
                state: PrState::Open,
                is_draft: false,
                head_ref: "chaosteil/kxyzkxyzkxyz".into(),
                head_oid: "abc123".into(),
                base_ref: "main".into(),
                additions: 10,
                deletions: 2,
                changed_files: 3,
                review_decision: Some(ReviewDecision::ReviewRequired),
                review_requested: true,
                reviewers: vec!["colleague".into()],
                approvals: 1,
                changes_requested: 2,
                stack: Some(StackRef {
                    id: "PRS_1".into(),
                    number: 7,
                    size: 2,
                    position: 1,
                    base_ref: "main".into(),
                }),
                auto_merge: false,
                in_merge_queue: false,
                mergeable: Mergeable::Unknown,
                merge_state: MergeStateStatus::Unknown,
                url: "https://github.com/chaosteil/chaoshub/pull/42".into(),
                updated_at: test_time(),
                repo_settings: RepoSettings {
                    squash: true,
                    merge: true,
                    rebase: false,
                    default_branch: None,
                    squash_title_default: None,
                    squash_message_default: None,
                },
            }),
            ci_state: observed(CiState::Pending),
            checks: observed(vec![Check {
                name: "lint".into(),
                kind: CheckKind::StatusContext,
                status: CheckStatus::Pending,
                conclusion: None,
                started: None,
                completed: None,
                url: None,
                run_id: None,
                suite_id: None,
            }]),
        }
    }

    #[test]
    fn pull_request_round_trip() {
        assert_round_trip(&pull_request());
    }

    #[test]
    fn an_old_cache_without_review_counts_loads_with_zero() {
        let mut json = serde_json::to_value(pull_request()).unwrap();
        let head = json["head"]["value"].as_object_mut().unwrap();
        assert!(head.remove("approvals").is_some());
        assert!(head.remove("changes_requested").is_some());
        let pr: PullRequest = serde_json::from_value(json).unwrap();
        assert_eq!(pr.head.value.approvals, 0);
        assert_eq!(pr.head.value.changes_requested, 0);
        assert_eq!(pr.head.value.reviewers, ["colleague"]);
    }
}
