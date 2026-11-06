//! Converts GraphQL responses into domain types.
//!
//! `graphql_client` generates one set of types per query module, so a
//! fragment that two queries share has two identical type trees. The
//! macros below write each conversion once and expand it in one module per
//! query. Inside a module, `q` names the generated query module.

use chrono::{DateTime, Utc};

use crate::domain::check::{Check, CheckConclusion, CheckKind, CheckStatus};
use crate::domain::keys::{PrKey, RepoKey};
use crate::domain::observed::Observed;
use crate::domain::pr::{
    CiState, MergeStateStatus, Mergeable, PrHead, PrState, PullRequest, ReviewDecision,
};
use crate::domain::repo::{RepoSettings, SquashMessageDefault, SquashTitleDefault};
use crate::domain::stack::{Stack, StackEntry, StackRef};
use crate::github::GithubError;

/// The user, for the `review_requested` rule.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Me<'a> {
    pub login: &'a str,
    /// Teams from the configuration, as `org/slug` or `slug`.
    pub teams: &'a [String],
}

impl Me<'_> {
    /// True when `org/slug` names one of the teams of the user.
    fn in_team(&self, org: &str, slug: &str) -> bool {
        self.teams.iter().any(|team| match team.split_once('/') {
            Some((team_org, team_slug)) => {
                team_org.eq_ignore_ascii_case(org) && team_slug.eq_ignore_ascii_case(slug)
            }
            None => team.eq_ignore_ascii_case(slug),
        })
    }
}

/// The CI state of one pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CiStatus {
    pub node_id: String,
    /// The head commit that the state belongs to.
    pub head_oid: String,
    pub state: Observed<CiState>,
}

/// The result of a CI poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CiPollResult {
    /// The CI state of every visible pull request that still exists.
    pub visible: Vec<CiStatus>,
    /// The state and the checks of the selected pull request.
    pub selected: Option<(CiStatus, Observed<Vec<Check>>)>,
}

/// Converts a GraphQL `Int` to an unsigned domain number.
fn uint<T: TryFrom<i64>>(value: i64, field: &str) -> Result<T, GithubError> {
    T::try_from(value).map_err(|_| GithubError::Parse(format!("{field} out of range: {value}")))
}

/// The repo key from `owner/name`.
fn repo_key(name_with_owner: &str) -> Result<RepoKey, GithubError> {
    let (owner, name) = name_with_owner
        .split_once('/')
        .ok_or_else(|| GithubError::Parse(format!("repo name without owner: {name_with_owner}")))?;
    Ok(RepoKey {
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}

/// Wraps `value` with the request start time.
fn observed<T>(value: T, observed_at: DateTime<Utc>) -> Observed<T> {
    Observed { value, observed_at }
}

/// Expands the conversion of `q::StatusState` and of the `RollupFields`
/// fragment (CI state and checks).
macro_rules! rollup_parser {
    () => {
        /// The combined CI state of a `StatusState`.
        pub(super) fn ci_state(state: &q::StatusState) -> CiState {
            match state {
                q::StatusState::SUCCESS => CiState::Success,
                q::StatusState::FAILURE | q::StatusState::ERROR => CiState::Failure,
                q::StatusState::PENDING | q::StatusState::EXPECTED | q::StatusState::Other(_) => {
                    CiState::Pending
                }
            }
        }

        /// The CI state and the checks of a rollup. No rollup means no checks.
        pub(super) fn rollup(rollup: Option<q::RollupFields>) -> (CiState, Vec<Check>) {
            let Some(rollup) = rollup else {
                return (CiState::None, Vec::new());
            };
            let checks = rollup
                .contexts
                .nodes
                .unwrap_or_default()
                .into_iter()
                .flatten()
                .map(check)
                .collect();
            (ci_state(&rollup.state), checks)
        }

        /// One check run or status context.
        fn check(node: q::RollupFieldsContextsNodes) -> Check {
            match node {
                q::RollupFieldsContextsNodes::CheckRun(run) => Check {
                    name: run.name,
                    kind: CheckKind::CheckRun,
                    status: match run.status {
                        q::CheckStatusState::COMPLETED => CheckStatus::Completed,
                        q::CheckStatusState::IN_PROGRESS => CheckStatus::InProgress,
                        q::CheckStatusState::QUEUED => CheckStatus::Queued,
                        q::CheckStatusState::REQUESTED => CheckStatus::Requested,
                        q::CheckStatusState::WAITING => CheckStatus::Waiting,
                        q::CheckStatusState::PENDING | q::CheckStatusState::Other(_) => {
                            CheckStatus::Pending
                        }
                    },
                    conclusion: run.conclusion.map(|conclusion| match conclusion {
                        q::CheckConclusionState::SUCCESS => CheckConclusion::Success,
                        q::CheckConclusionState::FAILURE | q::CheckConclusionState::Other(_) => {
                            CheckConclusion::Failure
                        }
                        q::CheckConclusionState::NEUTRAL => CheckConclusion::Neutral,
                        q::CheckConclusionState::CANCELLED => CheckConclusion::Cancelled,
                        q::CheckConclusionState::SKIPPED => CheckConclusion::Skipped,
                        q::CheckConclusionState::TIMED_OUT => CheckConclusion::TimedOut,
                        q::CheckConclusionState::ACTION_REQUIRED => CheckConclusion::ActionRequired,
                        q::CheckConclusionState::STARTUP_FAILURE => CheckConclusion::StartupFailure,
                        q::CheckConclusionState::STALE => CheckConclusion::Stale,
                    }),
                    started: run.started_at,
                    completed: run.completed_at,
                    url: run.details_url,
                    run_id: run
                        .check_suite
                        .workflow_run
                        .and_then(|workflow| workflow.database_id)
                        .and_then(|id| u64::try_from(id).ok()),
                    suite_id: run
                        .check_suite
                        .database_id
                        .and_then(|id| u64::try_from(id).ok()),
                },
                q::RollupFieldsContextsNodes::StatusContext(status) => {
                    let (check_status, conclusion) = match status.state {
                        q::StatusState::SUCCESS => {
                            (CheckStatus::Completed, Some(CheckConclusion::Success))
                        }
                        q::StatusState::FAILURE | q::StatusState::ERROR => {
                            (CheckStatus::Completed, Some(CheckConclusion::Failure))
                        }
                        q::StatusState::PENDING
                        | q::StatusState::EXPECTED
                        | q::StatusState::Other(_) => (CheckStatus::Pending, None),
                    };
                    Check {
                        name: status.context,
                        kind: CheckKind::StatusContext,
                        status: check_status,
                        conclusion,
                        started: None,
                        completed: None,
                        url: status.target_url,
                        run_id: None,
                        suite_id: None,
                    }
                }
            }
        }
    };
}

/// Expands the conversion of the `PrFields` fragment into a `PullRequest`.
/// The module also needs `rollup_parser!()`.
macro_rules! pr_fields_parser {
    () => {
        /// The merge settings of the repository of a pull request. The
        /// default branch is not in the fragment.
        fn repo_settings(repository: &q::PrFieldsRepository) -> RepoSettings {
            RepoSettings {
                squash: repository.squash_merge_allowed,
                merge: repository.merge_commit_allowed,
                rebase: repository.rebase_merge_allowed,
                default_branch: None,
                squash_title_default: match repository.squash_merge_commit_title {
                    q::SquashMergeCommitTitle::PR_TITLE => Some(SquashTitleDefault::PrTitle),
                    q::SquashMergeCommitTitle::COMMIT_OR_PR_TITLE => {
                        Some(SquashTitleDefault::CommitOrPrTitle)
                    }
                    q::SquashMergeCommitTitle::Other(_) => None,
                },
                squash_message_default: match repository.squash_merge_commit_message {
                    q::SquashMergeCommitMessage::PR_BODY => Some(SquashMessageDefault::PrBody),
                    q::SquashMergeCommitMessage::COMMIT_MESSAGES => {
                        Some(SquashMessageDefault::CommitMessages)
                    }
                    q::SquashMergeCommitMessage::BLANK => Some(SquashMessageDefault::Blank),
                    q::SquashMergeCommitMessage::Other(_) => None,
                },
            }
        }

        /// The latest opinionated reviews that approve and that request
        /// changes.
        fn review_counts(reviews: Option<q::PrFieldsLatestOpinionatedReviews>) -> (u32, u32) {
            let mut approvals = 0;
            let mut changes_requested = 0;
            for node in reviews
                .and_then(|reviews| reviews.nodes)
                .unwrap_or_default()
                .into_iter()
                .flatten()
            {
                match node.state {
                    q::PullRequestReviewState::APPROVED => approvals += 1,
                    q::PullRequestReviewState::CHANGES_REQUESTED => changes_requested += 1,
                    _ => {}
                }
            }
            (approvals, changes_requested)
        }

        /// A pull request from the `PrFields` fragment. Every field group
        /// carries `observed_at`, the start time of the request.
        #[allow(
            clippy::too_many_lines,
            reason = "the fragment maps to one pull request field by field"
        )]
        pub(super) fn pull_request(
            fields: q::PrFields,
            me: &Me<'_>,
            observed_at: DateTime<Utc>,
        ) -> Result<PullRequest, GithubError> {
            let key = PrKey {
                repo: repo_key(&fields.repository.name_with_owner)?,
                number: uint(fields.number, "number")?,
            };
            let mut reviewers = Vec::new();
            let mut requested_me = false;
            for node in fields
                .review_requests
                .and_then(|requests| requests.nodes)
                .unwrap_or_default()
                .into_iter()
                .flatten()
            {
                match node.requested_reviewer {
                    Some(q::PrFieldsReviewRequestsNodesRequestedReviewer::User(user)) => {
                        requested_me |= user.login.eq_ignore_ascii_case(me.login);
                        reviewers.push(user.login);
                    }
                    Some(q::PrFieldsReviewRequestsNodesRequestedReviewer::Team(team)) => {
                        requested_me |= me.in_team(&team.organization.login, &team.slug);
                        reviewers.push(format!("{}/{}", team.organization.login, team.slug));
                    }
                    _ => {}
                }
            }
            let (approvals, changes_requested) = review_counts(fields.latest_opinionated_reviews);
            let stack = match (fields.stack, fields.stack_entry) {
                (Some(stack), Some(entry)) => Some(StackRef {
                    id: stack.id,
                    number: uint(stack.number, "stack.number")?,
                    size: uint(stack.size, "stack.size")?,
                    position: uint(entry.position, "stackEntry.position")?,
                    base_ref: stack.base_ref_name,
                }),
                _ => None,
            };
            let head = PrHead {
                title: fields.title,
                body: fields.body,
                body_html: fields.body_html,
                author: fields
                    .author
                    .map_or_else(|| "ghost".to_owned(), |author| author.login),
                state: match fields.state {
                    q::PullRequestState::OPEN | q::PullRequestState::Other(_) => PrState::Open,
                    q::PullRequestState::CLOSED => PrState::Closed,
                    q::PullRequestState::MERGED => PrState::Merged,
                },
                is_draft: fields.is_draft,
                head_ref: fields.head_ref_name,
                head_oid: fields.head_ref_oid,
                base_ref: fields.base_ref_name,
                additions: uint(fields.additions, "additions")?,
                deletions: uint(fields.deletions, "deletions")?,
                changed_files: uint(fields.changed_files, "changedFiles")?,
                review_decision: fields.review_decision.and_then(|decision| match decision {
                    q::PullRequestReviewDecision::APPROVED => Some(ReviewDecision::Approved),
                    q::PullRequestReviewDecision::CHANGES_REQUESTED => {
                        Some(ReviewDecision::ChangesRequested)
                    }
                    q::PullRequestReviewDecision::REVIEW_REQUIRED => {
                        Some(ReviewDecision::ReviewRequired)
                    }
                    q::PullRequestReviewDecision::Other(_) => None,
                }),
                review_requested: requested_me || fields.viewer_latest_review_request.is_some(),
                reviewers,
                approvals,
                changes_requested,
                stack,
                auto_merge: fields.auto_merge_request.is_some(),
                in_merge_queue: fields.merge_queue_entry.is_some(),
                mergeable: match fields.mergeable {
                    q::MergeableState::MERGEABLE => Mergeable::Mergeable,
                    q::MergeableState::CONFLICTING => Mergeable::Conflicting,
                    q::MergeableState::UNKNOWN | q::MergeableState::Other(_) => Mergeable::Unknown,
                },
                merge_state: match fields.merge_state_status {
                    q::MergeStateStatus::BEHIND => MergeStateStatus::Behind,
                    q::MergeStateStatus::BLOCKED => MergeStateStatus::Blocked,
                    q::MergeStateStatus::CLEAN => MergeStateStatus::Clean,
                    q::MergeStateStatus::DIRTY => MergeStateStatus::Dirty,
                    q::MergeStateStatus::DRAFT => MergeStateStatus::Draft,
                    q::MergeStateStatus::HAS_HOOKS => MergeStateStatus::HasHooks,
                    q::MergeStateStatus::UNSTABLE => MergeStateStatus::Unstable,
                    q::MergeStateStatus::UNKNOWN | q::MergeStateStatus::Other(_) => {
                        MergeStateStatus::Unknown
                    }
                },
                url: fields.url,
                updated_at: fields.updated_at,
                repo_settings: repo_settings(&fields.repository),
            };
            let (ci_state, checks) = rollup(fields.status_check_rollup);
            Ok(PullRequest {
                key,
                node_id: fields.id,
                head: observed(head, observed_at),
                ci_state: observed(ci_state, observed_at),
                checks: observed(checks, observed_at),
            })
        }
    };
}

/// Conversions for the `ListPrs` query.
pub(crate) mod list_prs {
    #[allow(
        clippy::wildcard_imports,
        reason = "the macro bodies use the imports of the parent module"
    )]
    use super::*;
    use crate::github::queries::list_prs as q;

    rollup_parser!();
    pr_fields_parser!();

    /// The pull requests of one search page, and the cursor of the next
    /// page when there is one.
    pub(crate) fn page(
        data: q::ResponseData,
        me: &Me<'_>,
        observed_at: DateTime<Utc>,
    ) -> Result<(Vec<PullRequest>, Option<String>), GithubError> {
        let next = data
            .search
            .page_info
            .end_cursor
            .filter(|_| data.search.page_info.has_next_page);
        let prs = data
            .search
            .nodes
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .filter_map(|node| match node {
                q::ListPrsSearchNodes::PullRequest(fields) => Some(fields),
                _ => None,
            })
            .map(|fields| pull_request(fields, me, observed_at))
            .collect::<Result<_, _>>()?;
        Ok((prs, next))
    }
}

/// Conversions for the `PrByIds` query.
pub(crate) mod pr_by_ids {
    #[allow(
        clippy::wildcard_imports,
        reason = "the macro bodies use the imports of the parent module"
    )]
    use super::*;
    use crate::github::queries::pr_by_ids as q;

    rollup_parser!();
    pr_fields_parser!();

    /// The pull requests among the nodes. Missing nodes are skipped.
    pub(crate) fn prs(
        data: q::ResponseData,
        me: &Me<'_>,
        observed_at: DateTime<Utc>,
    ) -> Result<Vec<PullRequest>, GithubError> {
        data.nodes
            .into_iter()
            .flatten()
            .filter_map(|node| match node {
                q::PrByIdsNodes::PullRequest(fields) => Some(fields),
                _ => None,
            })
            .map(|fields| pull_request(fields, me, observed_at))
            .collect()
    }
}

/// Conversions for the `PrByNumber` query.
pub(crate) mod pr_by_number {
    #[allow(
        clippy::wildcard_imports,
        reason = "the macro bodies use the imports of the parent module"
    )]
    use super::*;
    use crate::github::queries::pr_by_number as q;

    rollup_parser!();
    pr_fields_parser!();

    /// The pull request. `None` when the repo or the pull request does not
    /// exist.
    pub(crate) fn pr(
        data: q::ResponseData,
        me: &Me<'_>,
        observed_at: DateTime<Utc>,
    ) -> Result<Option<PullRequest>, GithubError> {
        data.repository
            .and_then(|repo| repo.pull_request)
            .map(|fields| pull_request(fields, me, observed_at))
            .transpose()
    }
}

/// Conversions for the `StackEntries` query.
pub(crate) mod stack_entries {
    use super::{DateTime, GithubError, Observed, PrState, Stack, StackEntry, Utc, observed, uint};
    use crate::github::queries::stack_entries as q;

    /// The stack and its entries, sorted by position, position 1 first.
    /// Entries without a pull request are skipped. `None` when the node is
    /// not a stack.
    pub(crate) fn stack(
        data: q::ResponseData,
        observed_at: DateTime<Utc>,
    ) -> Result<Option<Observed<Stack>>, GithubError> {
        let Some(q::StackEntriesNode::PullRequestStack(node)) = data.node else {
            return Ok(None);
        };
        let mut entries = Vec::new();
        for entry in node.entries.nodes.unwrap_or_default().into_iter().flatten() {
            let Some(pr) = entry.pull_request else {
                continue;
            };
            entries.push(StackEntry {
                position: uint(entry.position, "position")?,
                number: uint(pr.number, "number")?,
                title: pr.title,
                state: match pr.state {
                    q::PullRequestState::OPEN | q::PullRequestState::Other(_) => PrState::Open,
                    q::PullRequestState::CLOSED => PrState::Closed,
                    q::PullRequestState::MERGED => PrState::Merged,
                },
                is_draft: pr.is_draft,
            });
        }
        entries.sort_by_key(|entry| entry.position);
        let stack = Stack {
            id: node.id,
            base_ref: node.base_ref_name,
            entries,
        };
        Ok(Some(observed(stack, observed_at)))
    }
}

/// Conversions for the `CiPoll` query.
pub(crate) mod ci_poll {
    #[allow(
        clippy::wildcard_imports,
        reason = "the macro bodies use the imports of the parent module"
    )]
    use super::*;
    use crate::github::queries::ci_poll as q;

    rollup_parser!();

    /// The CI states of the visible pull requests and the checks of the
    /// selected one. Nodes that are not pull requests are skipped.
    pub(crate) fn result(data: q::ResponseData, observed_at: DateTime<Utc>) -> CiPollResult {
        let visible = data
            .visible
            .into_iter()
            .flatten()
            .filter_map(|node| match node {
                q::CiPollVisible::PullRequest(pr) => Some(CiStatus {
                    node_id: pr.id,
                    head_oid: pr.head_ref_oid,
                    state: observed(
                        pr.status_check_rollup
                            .map_or(CiState::None, |rollup| ci_state(&rollup.state)),
                        observed_at,
                    ),
                }),
                _ => None,
            })
            .collect();
        let selected = match data.selected {
            Some(q::CiPollSelected::PullRequest(pr)) => {
                let (state, checks) = rollup(pr.status_check_rollup);
                Some((
                    CiStatus {
                        node_id: pr.id,
                        head_oid: pr.head_ref_oid,
                        state: observed(state, observed_at),
                    },
                    observed(checks, observed_at),
                ))
            }
            _ => None,
        };
        CiPollResult { visible, selected }
    }
}

/// Conversions for the `PrDetail` query.
pub(crate) mod pr_detail {
    use super::{DateTime, GithubError, Observed, Utc, observed, uint};
    use crate::domain::detail::{Comment, PrDetail, Review, ReviewState, Thread};
    use crate::github::queries::pr_detail as q;

    /// A `Comment` from a generated comment node. The issue comments and
    /// the thread comments have distinct types with the same fields.
    macro_rules! comment {
        ($node:expr) => {{
            let node = $node;
            Comment {
                author: node
                    .author
                    .map_or_else(|| "ghost".to_owned(), |author| author.login),
                body: node.body,
                body_html: node.body_html,
                created_at: node.created_at,
                url: node.url,
            }
        }};
    }

    /// The conversation of a pull request. `None` when the repo or the
    /// pull request does not exist.
    pub(crate) fn detail(
        data: q::ResponseData,
        observed_at: DateTime<Utc>,
    ) -> Result<Option<Observed<PrDetail>>, GithubError> {
        let Some(pr) = data.repository.and_then(|repo| repo.pull_request) else {
            return Ok(None);
        };
        let comments = pr
            .comments
            .nodes
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .map(|node| comment!(node))
            .collect();
        let reviews = pr
            .reviews
            .and_then(|reviews| reviews.nodes)
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .map(|node| Review {
                author: node
                    .author
                    .map_or_else(|| "ghost".to_owned(), |author| author.login),
                state: match node.state {
                    q::PullRequestReviewState::APPROVED => ReviewState::Approved,
                    q::PullRequestReviewState::CHANGES_REQUESTED => ReviewState::ChangesRequested,
                    q::PullRequestReviewState::COMMENTED | q::PullRequestReviewState::Other(_) => {
                        ReviewState::Commented
                    }
                    q::PullRequestReviewState::DISMISSED => ReviewState::Dismissed,
                    q::PullRequestReviewState::PENDING => ReviewState::Pending,
                },
                body: node.body,
                body_html: node.body_html,
                submitted_at: node.submitted_at,
            })
            .collect();
        let mut threads = Vec::new();
        for node in pr
            .review_threads
            .nodes
            .unwrap_or_default()
            .into_iter()
            .flatten()
        {
            threads.push(Thread {
                path: node.path,
                line: node.line.map(|line| uint(line, "line")).transpose()?,
                is_resolved: node.is_resolved,
                comments: node
                    .comments
                    .nodes
                    .unwrap_or_default()
                    .into_iter()
                    .flatten()
                    .map(|node| comment!(node))
                    .collect(),
            });
        }
        let detail = PrDetail {
            comments,
            reviews,
            threads,
            head_oid: pr.head_ref_oid,
        };
        Ok(Some(observed(detail, observed_at)))
    }
}

/// Decodes a captured fixture: the `data` of a GraphQL response.
#[cfg(test)]
pub(crate) fn fixture<T: serde::de::DeserializeOwned>(json: &str) -> T {
    let value: serde_json::Value = serde_json::from_str(json).unwrap();
    serde_json::from_value(value["data"].clone()).unwrap()
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::domain::test_time;

    const ME: Me<'static> = Me {
        login: "chaosteil",
        teams: &[],
    };

    fn page1() -> Vec<PullRequest> {
        let data = fixture(include_str!("testdata/list_prs_page1.json"));
        let (prs, next) = list_prs::page(data, &ME, test_time()).unwrap();
        assert_eq!(next.as_deref(), Some("Y3Vyc29yOjM="));
        prs
    }

    #[test]
    fn list_page_parses_every_pull_request() {
        let prs = page1();
        let numbers: Vec<u64> = prs.iter().map(|pr| pr.key.number).collect();
        assert_eq!(numbers, [1589, 1587, 1586]);
        for pr in &prs {
            assert_eq!(
                pr.key.repo,
                RepoKey {
                    owner: "acme".into(),
                    name: "deploy".into()
                }
            );
            assert_eq!(pr.head.observed_at, test_time());
            assert_eq!(pr.ci_state.observed_at, test_time());
            assert_eq!(pr.checks.observed_at, test_time());
        }
    }

    #[test]
    fn list_page_maps_the_head_fields() {
        let pr = page1().remove(2);
        assert!(pr.node_id.starts_with("PR_"));
        let head = &pr.head.value;
        assert_eq!(head.author, "chaosteil");
        assert_eq!(head.state, PrState::Merged);
        assert!(!head.is_draft);
        assert_eq!(head.head_ref, "dominykas/vsvzovrwyuss");
        assert_eq!(head.head_oid.len(), 40);
        assert_eq!(head.review_decision, Some(ReviewDecision::Approved));
        assert!(!head.review_requested);
        assert_eq!(head.url, "https://github.com/acme/deploy/pull/1586");
        assert_eq!(
            head.repo_settings,
            RepoSettings {
                squash: true,
                merge: false,
                rebase: true,
                default_branch: None,
                squash_title_default: Some(SquashTitleDefault::CommitOrPrTitle),
                squash_message_default: Some(SquashMessageDefault::CommitMessages),
            }
        );
        let stack = head.stack.as_ref().unwrap();
        assert_eq!((stack.number, stack.size, stack.position), (1528, 6, 6));
        assert_eq!(stack.id, "PRS_kwDORX-Rqc4AKSuA");
        assert_eq!(stack.base_ref, "main");
    }

    #[test]
    fn list_page_maps_checks_with_run_and_suite_ids() {
        let pr = page1().remove(0);
        assert_eq!(pr.ci_state.value, CiState::Success);
        let checks = &pr.checks.value;
        assert_eq!(checks.len(), 5);
        let run = checks
            .iter()
            .find(|check| check.kind == CheckKind::CheckRun)
            .unwrap();
        assert_eq!(run.status, CheckStatus::Completed);
        assert_eq!(run.conclusion, Some(CheckConclusion::Success));
        assert!(run.run_id.is_some());
        assert!(run.suite_id.is_some());
        assert!(run.started.is_some() && run.completed.is_some());
        let status = checks
            .iter()
            .find(|check| check.kind == CheckKind::StatusContext)
            .unwrap();
        assert_eq!(status.conclusion, Some(CheckConclusion::Success));
        assert_eq!(status.run_id, None);
    }

    #[test]
    fn team_page_maps_review_requests() {
        let data = fixture(include_str!("testdata/list_prs_team.json"));
        let (prs, _) = list_prs::page(data, &ME, test_time()).unwrap();
        let draft = &prs[0].head.value;
        assert!(draft.is_draft);
        assert_eq!(draft.state, PrState::Open);
        assert_ne!(draft.author, "chaosteil");
        assert!(draft.review_requested, "viewerLatestReviewRequest is set");
        assert_eq!(draft.reviewers, ["acme/team-1", "acme/team-2"]);
        assert_eq!(draft.review_decision, Some(ReviewDecision::ReviewRequired));
        assert_eq!(
            draft.repo_settings.squash_title_default,
            Some(SquashTitleDefault::PrTitle)
        );
        assert_eq!(
            draft.repo_settings.squash_message_default,
            Some(SquashMessageDefault::PrBody)
        );
        assert_eq!(prs[1].ci_state.value, CiState::Failure);
        assert!(
            prs[1]
                .checks
                .value
                .iter()
                .any(|check| check.conclusion == Some(CheckConclusion::Failure))
        );
    }

    #[test]
    fn list_page_counts_the_latest_opinionated_reviews() {
        assert_eq!(page1()[0].head.value.approvals, 1);
        assert_eq!(page1()[0].head.value.changes_requested, 0);
        let mut json: serde_json::Value =
            serde_json::from_str(include_str!("testdata/list_prs_page1.json")).unwrap();
        let review = |state: &str| {
            serde_json::json!({
                "author": { "__typename": "User", "login": "colleague" },
                "state": state,
            })
        };
        json["data"]["search"]["nodes"][0]["latestOpinionatedReviews"]["nodes"] =
            serde_json::json!([
                review("APPROVED"),
                review("CHANGES_REQUESTED"),
                review("APPROVED"),
                review("COMMENTED"),
                review("DISMISSED"),
                review("CHANGES_REQUESTED"),
                review("APPROVED"),
            ]);
        let data = fixture(&json.to_string());
        let (prs, _) = list_prs::page(data, &ME, test_time()).unwrap();
        let head = &prs[0].head.value;
        assert_eq!((head.approvals, head.changes_requested), (3, 2));
        let data = fixture(include_str!("testdata/list_prs_team.json"));
        let (prs, _) = list_prs::page(data, &ME, test_time()).unwrap();
        let head = &prs[0].head.value;
        assert_eq!((head.approvals, head.changes_requested), (0, 0));
    }

    #[test]
    fn me_matches_teams_with_and_without_org() {
        let teams = ["acme/team-1".to_owned(), "Team-2".to_owned()];
        let me = Me {
            login: "chaosteil",
            teams: &teams,
        };
        assert!(me.in_team("acme", "team-1"));
        assert!(me.in_team("ACME", "team-2"));
        assert!(!me.in_team("other", "team-1"));
        assert!(!me.in_team("acme", "team-3"));
    }

    #[test]
    fn by_ids_parses_the_nodes() {
        let data = fixture(include_str!("testdata/pr_by_ids.json"));
        let prs = pr_by_ids::prs(data, &ME, test_time()).unwrap();
        let numbers: Vec<u64> = prs.iter().map(|pr| pr.key.number).collect();
        assert_eq!(numbers, [1550, 1549]);
        assert_eq!(prs[0].head.value.state, PrState::Open);
        assert_eq!(prs[0].head.value.stack, None);
    }

    #[test]
    fn by_number_parses_the_pull_request() {
        let data = fixture(include_str!("testdata/pr_by_number.json"));
        let pr = pr_by_number::pr(data, &ME, test_time()).unwrap().unwrap();
        assert_eq!(pr.key.number, 1550);
        assert_eq!(pr.key.repo.owner, "acme");
        assert_eq!(pr.key.repo.name, "deploy");
        assert_eq!(pr.head.value.state, PrState::Open);
        assert_eq!(pr.head.observed_at, test_time());
    }

    #[test]
    fn by_number_of_a_null_pull_request_is_none() {
        for data in [
            serde_json::json!({ "repository": { "pullRequest": null }, "rateLimit": null }),
            serde_json::json!({ "repository": null, "rateLimit": null }),
        ] {
            let data = serde_json::from_value(data).unwrap();
            assert_eq!(pr_by_number::pr(data, &ME, test_time()).unwrap(), None);
        }
    }

    #[test]
    fn stack_entries_are_sorted_by_position() {
        let mut json: serde_json::Value =
            serde_json::from_str(include_str!("testdata/stack_entries.json")).unwrap();
        let nodes = json["data"]["node"]["entries"]["nodes"]
            .as_array_mut()
            .unwrap();
        nodes.reverse();
        nodes.push(serde_json::json!({ "position": 7, "pullRequest": null }));
        let data = serde_json::from_value(json["data"].clone()).unwrap();
        let stack = stack_entries::stack(data, test_time()).unwrap().unwrap();
        assert_eq!(stack.observed_at, test_time());
        assert_eq!(stack.value.id, "PRS_kwDORX-Rqc4AKSuA");
        assert_eq!(stack.value.base_ref, "main");
        let positions: Vec<(u32, u64)> = stack
            .value
            .entries
            .iter()
            .map(|entry| (entry.position, entry.number))
            .collect();
        assert_eq!(
            positions,
            [
                (1, 1523),
                (2, 1524),
                (3, 1525),
                (4, 1526),
                (5, 1527),
                (6, 1586)
            ]
        );
        assert_eq!(stack.value.entries[0].state, PrState::Merged);
        assert!(!stack.value.entries[0].is_draft);
    }

    #[test]
    fn stack_entries_of_another_node_is_none() {
        let data = serde_json::from_value(serde_json::json!({
            "node": { "__typename": "PullRequest" },
            "rateLimit": null
        }))
        .unwrap();
        assert_eq!(stack_entries::stack(data, test_time()).unwrap(), None);
    }

    #[test]
    fn ci_poll_maps_visible_states_and_selected_checks() {
        let data = fixture(include_str!("testdata/ci_poll.json"));
        let result = ci_poll::result(data, test_time());
        let states: Vec<CiState> = result
            .visible
            .iter()
            .map(|status| status.state.value)
            .collect();
        assert_eq!(states, [CiState::Success, CiState::Failure]);
        assert!(
            result
                .visible
                .iter()
                .all(|status| status.head_oid.len() == 40)
        );
        assert!(
            result
                .visible
                .iter()
                .all(|status| status.state.observed_at == test_time())
        );
        let (selected, checks) = result.selected.unwrap();
        assert_eq!(selected.node_id, result.visible[1].node_id);
        assert_eq!(selected.state.value, CiState::Failure);
        assert_eq!(checks.observed_at, test_time());
        assert_eq!(checks.value.len(), 19);
        let failed: Vec<&Check> = checks
            .value
            .iter()
            .filter(|check| check.conclusion == Some(CheckConclusion::Failure))
            .collect();
        assert!(
            failed
                .iter()
                .all(|check| check.run_id.is_some() && check.suite_id.is_some())
        );
        assert_eq!(failed.len(), 1);
    }

    #[test]
    fn ci_poll_without_selection_or_rollup() {
        let data = serde_json::from_value(serde_json::json!({
            "visible": [
                { "__typename": "PullRequest", "id": "PR_1", "headRefOid": "abc", "statusCheckRollup": null },
                null
            ],
            "rateLimit": null
        }))
        .unwrap();
        let result = ci_poll::result(data, test_time());
        assert_eq!(result.visible.len(), 1);
        assert_eq!(result.visible[0].state.value, CiState::None);
        assert_eq!(result.selected, None);
    }

    #[test]
    fn pr_detail_maps_comments_reviews_and_threads() {
        use crate::domain::detail::ReviewState;

        let data = fixture(include_str!("testdata/pr_detail.json"));
        let detail = pr_detail::detail(data, test_time()).unwrap().unwrap();
        assert_eq!(detail.observed_at, test_time());
        let detail = detail.value;
        assert_eq!(detail.head_oid, "1e2defa1ebcdb1cb5232604e4ba6000c00d2d0e2");
        assert_eq!(detail.comments.len(), 2);
        assert_eq!(detail.comments[0].author, "vercel");
        assert!(detail.comments[0].url.ends_with("#issuecomment-6070378109"));
        assert_eq!(detail.reviews.len(), 3);
        assert_eq!(detail.reviews[0].state, ReviewState::Commented);
        assert!(detail.reviews[0].body.starts_with("Body"));
        assert!(detail.reviews[1].submitted_at.is_some());
        let lines: Vec<(bool, Option<u32>, usize)> = detail
            .threads
            .iter()
            .map(|thread| (thread.is_resolved, thread.line, thread.comments.len()))
            .collect();
        assert_eq!(lines, [(true, None, 2), (false, Some(550), 2)]);
        assert_eq!(detail.threads[0].path, "src/file-1.rs");
    }

    #[test]
    fn pr_detail_of_a_missing_pull_request_is_none() {
        let data = serde_json::from_value(serde_json::json!({
            "repository": { "pullRequest": null },
            "rateLimit": null
        }))
        .unwrap();
        assert_eq!(pr_detail::detail(data, test_time()).unwrap(), None);
    }

    #[test]
    fn rollup_without_data_is_none() {
        assert_eq!(list_prs::rollup(None), (CiState::None, Vec::new()));
    }

    #[test]
    fn uint_rejects_negative_numbers() {
        assert!(matches!(uint::<u64>(-1, "n"), Err(GithubError::Parse(_))));
        assert_eq!(uint::<u32>(7, "n").unwrap(), 7);
    }
}
