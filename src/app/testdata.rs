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

/// A pull request of `chaoshub` in the stack `S_1` (#12, 3 entries) at
/// `position`.
pub(crate) fn stacked(number: u64, position: u32, title: &str) -> PullRequest {
    let mut pr = pr("chaoshub", number, title);
    pr.head.value.stack = Some(crate::domain::stack::StackRef {
        id: "S_1".into(),
        number: 12,
        size: 3,
        position,
        base_ref: "main".into(),
    });
    pr
}

/// The pull requests of the user: a 3-PR stack with mixed CI and review
/// states, and two single pull requests. Newest update first: #99, the
/// stack, #7.
pub(crate) fn mine_prs() -> Vec<PullRequest> {
    use crate::domain::pr::ReviewDecision;

    let mut base = stacked(101, 1, "feat(app): Pane trait, registry and overlay stack");
    base.ci_state.value = CiState::Failure;
    base.head.value.review_decision = Some(ReviewDecision::ChangesRequested);
    base.head.value.mergeable = Mergeable::Conflicting;
    let mut middle = stacked(102, 2, "feat(app): checks pane");
    middle.ci_state.value = CiState::Success;
    middle.head.value.review_decision = Some(ReviewDecision::Approved);
    let mut top = stacked(103, 3, "feat(ui): popup frame with pane tabs");
    top.ci_state.value = CiState::Pending;
    top.head.value.review_decision = Some(ReviewDecision::ReviewRequired);
    top.head.value.is_draft = true;
    let mut single = pr(
        "chaoshub",
        99,
        "fix(store): reopen the connection after an error",
    );
    single.head.value.updated_at += chrono::TimeDelta::hours(2);
    let mut old = pr(
        "codeview",
        7,
        "docs: describe the export file format in the README",
    );
    old.head.value.updated_at -= chrono::TimeDelta::days(3);
    old.ci_state.value = CiState::Success;
    vec![base, middle, top, single, old]
}

/// A startup snapshot with the pull requests of `mine_prs`.
pub(crate) fn mine_snapshot() -> crate::store::Snapshot {
    crate::store::Snapshot {
        prs: mine_prs()
            .into_iter()
            .map(|pr| (crate::bus::Scope::Mine, pr))
            .collect(),
        synced_at: Some(test_time()),
        ..crate::store::Snapshot::default()
    }
}
