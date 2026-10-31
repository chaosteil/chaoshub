//! Matches pull requests to local changes and groups stacks.

use std::collections::{HashSet, VecDeque};

use crate::domain::keys::PrKey;
use crate::domain::local::{LocalChange, LocalMatch};
use crate::domain::pr::PullRequest;
use crate::domain::stack::StackGroup;

/// The shortest branch suffix that counts as a change id prefix.
const MIN_CHANGE_PREFIX: usize = 8;

/// Matches each pull request to a local change of its repo. The rules, in
/// this order:
///
/// 1. The head commit equals the commit of a change: `Synced`.
/// 2. The head branch is a bookmark of a change: `Synced` when the commit
///    equals the head commit, else `Diverged`.
/// 3. The last `/` segment of the head branch is a prefix of a change id
///    (at least 8 letters of the jj alphabet `k` to `z`): the same test.
///
/// Every pull request gets one entry; `LocalMatch::None` when no rule
/// matches. `changes` are the changes of the repo of the pull requests.
pub(crate) fn match_prs(prs: &[&PullRequest], changes: &[LocalChange]) -> Vec<(PrKey, LocalMatch)> {
    prs.iter()
        .map(|pr| (pr.key.clone(), match_pr(pr, changes)))
        .collect()
}

/// The match of one pull request.
fn match_pr(pr: &PullRequest, changes: &[LocalChange]) -> LocalMatch {
    let head = &pr.head.value;
    if let Some(change) = changes
        .iter()
        .find(|change| same_commit(&change.commit_id, &head.head_oid))
    {
        return synced(change);
    }
    if let Some(change) = changes
        .iter()
        .find(|change| change.bookmarks.contains(&head.head_ref))
    {
        return compare(change, &head.head_oid);
    }
    let suffix = head.head_ref.rsplit('/').next().unwrap_or_default();
    if is_change_prefix(suffix)
        && let Some(change) = changes
            .iter()
            .find(|change| change.change_id.starts_with(suffix))
    {
        return compare(change, &head.head_oid);
    }
    LocalMatch::None
}

/// The change id of a match, if any.
fn matched_change(m: &LocalMatch) -> Option<&str> {
    match m {
        LocalMatch::None => None,
        LocalMatch::Synced { change_id, .. } | LocalMatch::Diverged { change_id, .. } => {
            Some(change_id)
        }
    }
}

/// The changes that no pull request matches, once per change id, in
/// the order of `changes`. A change that is empty and has no description
/// is left out: it is a fresh working copy, not work.
pub(crate) fn local_only(
    changes: &[LocalChange],
    matches: &[(PrKey, LocalMatch)],
) -> Vec<LocalChange> {
    let taken: HashSet<&str> = matches
        .iter()
        .filter_map(|(_, m)| matched_change(m))
        .collect();
    let mut seen = HashSet::new();
    changes
        .iter()
        .filter(|change| !taken.contains(change.change_id.as_str()))
        .filter(|change| !(change.empty && change.description.is_empty()))
        .filter(|change| seen.insert(change.change_id.clone()))
        .cloned()
        .collect()
}

/// The pull request of the nearest ancestor of `change` that a pull
/// request matches. The search follows the parents through `changes`,
/// breadth first. `None` when no ancestor in `changes` has a pull
/// request.
pub(crate) fn nearest_pr(
    change: &LocalChange,
    changes: &[LocalChange],
    matches: &[(PrKey, LocalMatch)],
) -> Option<PrKey> {
    let pr_of = |id: &str| {
        matches
            .iter()
            .find(|(_, m)| matched_change(m) == Some(id))
            .map(|(key, _)| key.clone())
    };
    let mut queue: VecDeque<&str> = change.parents.iter().map(String::as_str).collect();
    let mut seen: HashSet<&str> = HashSet::new();
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(key) = pr_of(id) {
            return Some(key);
        }
        if let Some(parent) = changes.iter().find(|c| c.change_id == id) {
            queue.extend(parent.parents.iter().map(String::as_str));
        }
    }
    None
}

/// True when two commit ids name the same commit. One of them can be a
/// short form of the other.
fn same_commit(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a.starts_with(b) || b.starts_with(a))
}

/// True for a text that can be a change id prefix: at least 8 letters,
/// each from `k` to `z`.
fn is_change_prefix(text: &str) -> bool {
    text.len() >= MIN_CHANGE_PREFIX && text.chars().all(|c| ('k'..='z').contains(&c))
}

/// `Synced` for a change.
fn synced(change: &LocalChange) -> LocalMatch {
    LocalMatch::Synced {
        workspace: change.workspace.clone(),
        change_id: change.change_id.clone(),
        commit_id: change.commit_id.clone(),
    }
}

/// `Synced` when the change has the head commit, else `Diverged`.
fn compare(change: &LocalChange, head_oid: &str) -> LocalMatch {
    if same_commit(&change.commit_id, head_oid) {
        return synced(change);
    }
    LocalMatch::Diverged {
        workspace: change.workspace.clone(),
        change_id: change.change_id.clone(),
        commit_id: change.commit_id.clone(),
    }
}

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
pub(crate) mod tests {
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

    fn change(
        workspace: &str,
        change_id: &str,
        commit_id: &str,
        bookmarks: &[&str],
    ) -> LocalChange {
        LocalChange {
            workspace: workspace.into(),
            change_id: change_id.into(),
            commit_id: commit_id.into(),
            bookmarks: bookmarks.iter().map(|b| (*b).to_owned()).collect(),
            empty: false,
            parents: Vec::new(),
            description: "a change".into(),
        }
    }

    /// A pull request with a head branch and a head commit.
    fn headed(head_ref: &str, head_oid: &str) -> PullRequest {
        let mut pr = pr(1, "", 0);
        pr.head.value.head_ref = head_ref.into();
        pr.head.value.head_oid = head_oid.into();
        pr
    }

    #[test]
    fn match_prs_cases() {
        let synced = |ws: &str, id: &str, commit: &str| LocalMatch::Synced {
            workspace: ws.into(),
            change_id: id.into(),
            commit_id: commit.into(),
        };
        let diverged = |ws: &str, id: &str, commit: &str| LocalMatch::Diverged {
            workspace: ws.into(),
            change_id: id.into(),
            commit_id: commit.into(),
        };
        let changes = vec![
            change("default", "kxyzkxyzkxyzmmmm", "aaa111", &[]),
            change("feature", "qrstqrstqrstnnnn", "bbb222", &["topic"]),
            change("feature", "wwwwvvvvuuuu0000", "ccc333", &[]),
        ];
        let cases = [
            (
                "same commit",
                headed("x/y", "aaa111"),
                synced("default", "kxyzkxyzkxyzmmmm", "aaa111"),
            ),
            (
                "short commit id",
                headed("x/y", "aaa1"),
                synced("default", "kxyzkxyzkxyzmmmm", "aaa111"),
            ),
            (
                "bookmark, same commit",
                headed("topic", "bbb222"),
                synced("feature", "qrstqrstqrstnnnn", "bbb222"),
            ),
            (
                "bookmark, other commit",
                headed("topic", "fff999"),
                diverged("feature", "qrstqrstqrstnnnn", "bbb222"),
            ),
            (
                "change id suffix, home prefix",
                headed("chaosteil/kxyzkxyzkxyz", "fff999"),
                diverged("default", "kxyzkxyzkxyzmmmm", "aaa111"),
            ),
            (
                "change id suffix, work prefix",
                headed("dominykas/qrstqrst", "fff999"),
                diverged("feature", "qrstqrstqrstnnnn", "bbb222"),
            ),
            (
                "suffix too short",
                headed("chaosteil/kxyzkxy", "fff999"),
                LocalMatch::None,
            ),
            (
                "suffix outside the jj alphabet",
                headed("chaosteil/wwwwvvvvuuuu0", "fff999"),
                LocalMatch::None,
            ),
            ("no match", headed("main-fix", "fff999"), LocalMatch::None),
            ("empty head oid", headed("main-fix", ""), LocalMatch::None),
        ];
        for (name, pr, expected) in cases {
            assert_eq!(
                match_prs(&[&pr], &changes),
                vec![(pr.key.clone(), expected)],
                "{name}"
            );
        }
    }

    #[test]
    fn the_commit_rule_comes_before_the_bookmark_rule() {
        let changes = vec![
            change("one", "kkkkkkkkkkkk", "aaa111", &["topic"]),
            change("two", "llllllllllll", "bbb222", &[]),
        ];
        let pr = headed("topic", "bbb222");
        assert_eq!(
            match_prs(&[&pr], &changes)[0].1,
            LocalMatch::Synced {
                workspace: "two".into(),
                change_id: "llllllllllll".into(),
                commit_id: "bbb222".into(),
            }
        );
    }

    /// A change of the `ws` stack with parents and an empty flag.
    fn node(id: &str, parents: &[&str], empty: bool, description: &str) -> LocalChange {
        LocalChange {
            parents: parents.iter().map(|p| (*p).to_owned()).collect(),
            empty,
            description: description.into(),
            ..change("ws", id, &format!("c_{id}"), &[])
        }
    }

    fn synced_to(id: &str) -> LocalMatch {
        LocalMatch::Synced {
            workspace: "ws".into(),
            change_id: id.into(),
            commit_id: format!("c_{id}"),
        }
    }

    #[test]
    fn local_only_cases() {
        // A stack: base <- pr1 <- wip <- top, and a fresh empty working copy.
        let changes = vec![
            node("fresh", &["top"], true, ""),
            node("top", &["wip"], false, "top"),
            node("wip", &["pr1"], true, "wip: an empty change with a note"),
            node("pr1", &["base"], false, "the pull request"),
            node("base", &[], false, "base"),
            node("top", &["wip"], false, "top, again in another workspace"),
        ];
        let matches = vec![(key(1), synced_to("pr1")), (key(2), LocalMatch::None)];
        let ids: Vec<String> = local_only(&changes, &matches)
            .into_iter()
            .map(|c| c.change_id)
            .collect();
        assert_eq!(ids, ["top", "wip", "base"]);
    }

    #[test]
    fn nearest_pr_cases() {
        let changes = vec![
            node("top", &["wip"], false, "top"),
            node("wip", &["pr2"], false, "wip"),
            node("pr2", &["pr1"], false, "second"),
            node("pr1", &["base"], false, "first"),
            node("base", &[], false, "base"),
            node("merge", &["base", "pr1"], false, "merge"),
            node("loop", &["loop"], false, "a parent cycle"),
        ];
        let matches = vec![(key(1), synced_to("pr1")), (key(2), synced_to("pr2"))];
        let anchor = |id: &str| {
            let change = changes.iter().find(|c| c.change_id == id).unwrap();
            nearest_pr(change, &changes, &matches).map(|k| k.number)
        };
        let cases = [
            ("top", Some(2)),
            ("wip", Some(2)),
            ("pr2", Some(1)),
            ("base", None),
            ("merge", Some(1)),
            ("loop", None),
        ];
        for (id, expected) in cases {
            assert_eq!(anchor(id), expected, "{id}");
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
