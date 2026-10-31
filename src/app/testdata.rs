//! Fixtures for the tests of `app`: pull requests and seeded states.

use crate::domain::keys::{PrKey, RepoKey};
use crate::domain::observed::Observed;
use crate::domain::pr::{CiState, MergeStateStatus, Mergeable, PrHead, PrState, PullRequest};
use crate::domain::repo::RepoSettings;
use crate::domain::test_time;

/// A value observed at the test time.
pub(crate) fn observed<T>(value: T) -> Observed<T> {
    Observed {
        value,
        observed_at: test_time(),
    }
}

/// The key of the repo `chaosteil/<name>`.
pub(crate) fn repo_key(name: &str) -> RepoKey {
    RepoKey {
        owner: "chaosteil".into(),
        name: name.into(),
    }
}

/// The key of a pull request of `chaosteil/<repo>`.
pub(crate) fn pr_key(repo: &str, number: u64) -> PrKey {
    PrKey {
        repo: repo_key(repo),
        number,
    }
}

/// An open pull request of `chaosteil/<repo>` by `chaosteil`, observed at
/// the test time, without a stack and without checks.
pub(crate) fn pr(repo: &str, number: u64, title: &str) -> PullRequest {
    PullRequest {
        key: pr_key(repo, number),
        node_id: format!("PR_{repo}_{number}"),
        head: observed(PrHead {
            title: title.into(),
            body: String::new(),
            author: "chaosteil".into(),
            state: PrState::Open,
            is_draft: false,
            head_ref: format!("chaosteil/{number}"),
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
            url: format!("https://github.com/chaosteil/{repo}/pull/{number}"),
            updated_at: test_time(),
            repo_settings: RepoSettings {
                squash: true,
                merge: true,
                rebase: true,
                default_branch: None,
            },
        }),
        ci_state: observed(CiState::None),
        checks: observed(Vec::new()),
    }
}
