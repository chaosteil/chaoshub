//! Matches pull requests to local changes and groups stacks.

use crate::domain::keys::PrKey;
use crate::domain::local::{LocalChange, LocalMatch, Vcs};
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
///    Only jj changes take part in this rule.
///
/// For a git branch, the commit is the tip of the branch and the bookmark
/// is the branch name. So a branch matches when its tip is the head
/// commit, or when its name is the head branch.
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
            .find(|change| change.vcs == Vcs::Jj && change.change_id.starts_with(suffix))
    {
        return compare(change, &head.head_oid);
    }
    LocalMatch::None
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
        vcs: change.vcs,
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
        vcs: change.vcs,
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
                body_html: String::new(),
                author: "me".into(),
                state: PrState::Open,
                is_draft: false,
                head_ref: format!("me/{number}"),
                head_oid: format!("oid{number}"),
                base_ref: "main".into(),
                base_oid: String::new(),
                head_repo: String::new(),
                cross_repo: false,
                additions: 0,
                deletions: 0,
                changed_files: 0,
                review_decision: None,
                review_requested: false,
                reviewers: Vec::new(),
                approvals: 0,
                changes_requested: 0,
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
                created_at: Some(test_time()),
                updated_at: test_time(),
                repo_settings: RepoSettings {
                    squash: true,
                    merge: true,
                    rebase: true,
                    default_branch: None,
                    squash_title_default: None,
                    squash_message_default: None,
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
            vcs: Vcs::Jj,
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
            vcs: Vcs::Jj,
        };
        let diverged = |ws: &str, id: &str, commit: &str| LocalMatch::Diverged {
            workspace: ws.into(),
            change_id: id.into(),
            commit_id: commit.into(),
            vcs: Vcs::Jj,
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
                vcs: Vcs::Jj,
            }
        );
    }

    /// A git branch, checked out in `worktree`.
    fn branch(worktree: &str, name: &str, tip: &str) -> LocalChange {
        LocalChange {
            workspace: worktree.into(),
            change_id: name.into(),
            commit_id: tip.into(),
            bookmarks: vec![name.into()],
            empty: false,
            parents: Vec::new(),
            description: "a commit".into(),
            vcs: Vcs::Git,
        }
    }

    #[test]
    fn git_branches_match_by_tip_or_by_name() {
        let git = |id: &str, commit: &str, synced: bool| {
            let (workspace, change_id, commit_id) = ("default".into(), id.into(), commit.into());
            if synced {
                LocalMatch::Synced {
                    workspace,
                    change_id,
                    commit_id,
                    vcs: Vcs::Git,
                }
            } else {
                LocalMatch::Diverged {
                    workspace,
                    change_id,
                    commit_id,
                    vcs: Vcs::Git,
                }
            }
        };
        let changes = vec![
            branch("default", "me/topic", "aaa111"),
            branch("default", "kxyzkxyzkxyz", "bbb222"),
        ];
        let cases = [
            (
                "tip commit",
                headed("other", "aaa111"),
                git("me/topic", "aaa111", true),
            ),
            (
                "branch name, other commit",
                headed("me/topic", "fff999"),
                git("me/topic", "aaa111", false),
            ),
            (
                "the change id rule is jj only",
                headed("chaosteil/kxyzkxyz", "fff999"),
                LocalMatch::None,
            ),
        ];
        for (name, pr, expected) in cases {
            assert_eq!(match_prs(&[&pr], &changes)[0].1, expected, "{name}");
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
