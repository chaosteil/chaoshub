//! Matches pull requests to local changes and groups stacks.

use crate::domain::pr::PullRequest;
use crate::domain::stack::StackGroup;

/// Groups pull requests by stack. Pull requests with the same `stack.id`
/// form one group, top of the stack (highest position) first. A pull
/// request without a stack is a group of its own. The groups keep the
/// order of the first pull request of each group in `prs`.
pub(crate) fn group_stacks(prs: &[&PullRequest]) -> Vec<StackGroup> {
    let mut groups: Vec<(StackGroup, Vec<u32>)> = Vec::new();
    for pr in prs {
        let head = &pr.head.value;
        let found = head.stack.as_ref().and_then(|stack| {
            groups.iter_mut().find(|(group, _)| {
                group
                    .stack
                    .as_ref()
                    .is_some_and(|other| other.id == stack.id)
            })
        });
        let position = head.stack.as_ref().map_or(0, |stack| stack.position);
        match found {
            Some((group, positions)) => {
                group.prs.push(pr.key.clone());
                positions.push(position);
            }
            None => groups.push((
                StackGroup {
                    stack: head.stack.clone(),
                    prs: vec![pr.key.clone()],
                },
                vec![position],
            )),
        }
    }
    groups
        .into_iter()
        .map(|(mut group, positions)| {
            let mut order: Vec<_> = positions.into_iter().zip(group.prs).collect();
            order.sort_by_key(|(position, _)| std::cmp::Reverse(*position));
            group.prs = order.into_iter().map(|(_, key)| key).collect();
            group
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::keys::{PrKey, RepoKey};
    use crate::domain::observed::Observed;
    use crate::domain::pr::{CiState, MergeStateStatus, Mergeable, PrHead, PrState};
    use crate::domain::repo::RepoSettings;
    use crate::domain::stack::StackRef;
    use crate::domain::test_time;

    fn observed<T>(value: T) -> Observed<T> {
        Observed {
            value,
            observed_at: test_time(),
        }
    }

    fn key(number: u64) -> PrKey {
        PrKey {
            repo: RepoKey {
                owner: "o".into(),
                name: "r".into(),
            },
            number,
        }
    }

    /// A pull request, in the stack `stack` at `position` when `stack` is
    /// not empty.
    pub(crate) fn pr(number: u64, stack: &str, position: u32) -> PullRequest {
        PullRequest {
            key: key(number),
            node_id: format!("PR_{number}"),
            head: observed(PrHead {
                title: format!("Change {number}"),
                body: String::new(),
                author: "me".into(),
                state: PrState::Open,
                is_draft: false,
                head_ref: format!("me/{number}"),
                head_oid: format!("oid{number}"),
                base_ref: "main".into(),
                review_decision: None,
                review_requested: false,
                reviewers: Vec::new(),
                stack: (!stack.is_empty()).then(|| StackRef {
                    id: stack.into(),
                    number: 1,
                    size: 3,
                    position,
                    base_ref: "main".into(),
                }),
                auto_merge: false,
                in_merge_queue: false,
                mergeable: Mergeable::Unknown,
                merge_state: MergeStateStatus::Unknown,
                url: String::new(),
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

    /// The stack id and the numbers of each group.
    type Groups = Vec<(Option<&'static str>, Vec<u64>)>;

    #[test]
    fn group_stacks_cases() {
        let cases: [(&str, Vec<PullRequest>, Groups); 4] = [
            ("empty", vec![], vec![]),
            (
                "single pull requests keep their order",
                vec![pr(2, "", 0), pr(1, "", 0)],
                vec![(None, vec![2]), (None, vec![1])],
            ),
            (
                "a stack sorts by position, top first",
                vec![pr(11, "S", 1), pr(13, "S", 3), pr(12, "S", 2)],
                vec![(Some("S"), vec![13, 12, 11])],
            ),
            (
                "stacks and singles mixed, in first-seen order",
                vec![
                    pr(5, "", 0),
                    pr(21, "T", 1),
                    pr(11, "S", 1),
                    pr(22, "T", 2),
                    pr(6, "", 0),
                ],
                vec![
                    (None, vec![5]),
                    (Some("T"), vec![22, 21]),
                    (Some("S"), vec![11]),
                    (None, vec![6]),
                ],
            ),
        ];
        for (name, prs, expected) in cases {
            let refs: Vec<&PullRequest> = prs.iter().collect();
            let grouped = group_stacks(&refs);
            let groups: Vec<(Option<&str>, Vec<u64>)> = grouped
                .iter()
                .map(|group| {
                    (
                        group.stack.as_ref().map(|stack| stack.id.as_str()),
                        group.prs.iter().map(|key| key.number).collect(),
                    )
                })
                .collect();
            assert_eq!(groups, expected, "{name}");
        }
    }
}
