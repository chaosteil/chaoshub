//! Merge method, squash message and merge blockers.

use serde::{Deserialize, Serialize};

use crate::domain::pr::{MergeStateStatus, Mergeable, PullRequest};
use crate::domain::repo::Repo;

/// A GitHub merge method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    /// The value of the `merge_method` field of the merge request.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            MergeMethod::Merge => "merge",
            MergeMethod::Squash => "squash",
            MergeMethod::Rebase => "rebase",
        }
    }
}

/// The merge method for a repo: the override of the configuration, else
/// squash when the repo permits it, else merge, else rebase. Without
/// known settings it is squash.
pub(crate) fn resolve_method(repo: &Repo, method: Option<MergeMethod>) -> MergeMethod {
    if let Some(method) = method {
        return method;
    }
    match &repo.settings {
        None => MergeMethod::Squash,
        Some(settings) if settings.squash => MergeMethod::Squash,
        Some(settings) if settings.merge => MergeMethod::Merge,
        Some(_) => MergeMethod::Rebase,
    }
}

/// The commit title and body of a squash merge: `<title> (#N)`, and the
/// pull request body with CRLF line ends turned into LF and the trailing
/// whitespace removed.
pub(crate) fn squash_message(pr: &PullRequest) -> (String, String) {
    let head = &pr.head.value;
    let title = format!("{} (#{})", head.title.trim(), pr.key.number);
    let body = head.body.replace("\r\n", "\n").trim_end().to_owned();
    (title, body)
}

/// What stops a merge, and what the user must know before it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Blockers {
    /// Reasons that disable the merge.
    pub hard: Vec<String>,
    /// Reasons that show in the confirm popup. GitHub checks its rules
    /// itself and reports a failure.
    pub warnings: Vec<String>,
}

/// The blockers of a merge over `entries`: the stack entries up to the
/// merged position, or the pull request alone. Hard: a draft or a
/// conflict on any entry. Warnings: a `BLOCKED` merge state, and a merge
/// state that GitHub did not compute yet (`UNKNOWN`).
pub(crate) fn merge_blockers(entries: &[PullRequest]) -> Blockers {
    let mut blockers = Blockers::default();
    for pr in entries {
        let head = &pr.head.value;
        let number = pr.key.number;
        if head.is_draft {
            blockers.hard.push(format!("#{number} is a draft"));
        }
        if head.mergeable == Mergeable::Conflicting {
            blockers.hard.push(format!("#{number} has conflicts"));
        }
        if head.merge_state == MergeStateStatus::Blocked {
            blockers
                .warnings
                .push(format!("#{number} is blocked by the branch rules"));
        }
        if head.mergeable == Mergeable::Unknown || head.merge_state == MergeStateStatus::Unknown {
            blockers
                .warnings
                .push(format!("#{number} merge state is not known yet"));
        }
    }
    blockers
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::keys::RepoKey;
    use crate::domain::matching::tests::pr;
    use crate::domain::repo::RepoSettings;

    fn repo(settings: Option<(bool, bool, bool)>) -> Repo {
        Repo {
            key: RepoKey {
                owner: "o".into(),
                name: "r".into(),
            },
            path: None,
            settings: settings.map(|(squash, merge, rebase)| RepoSettings {
                squash,
                merge,
                rebase,
                default_branch: None,
            }),
        }
    }

    #[test]
    fn resolve_method_cases() {
        let cases = [
            (
                "override wins",
                repo(Some((true, true, true))),
                Some(MergeMethod::Rebase),
                MergeMethod::Rebase,
            ),
            (
                "squash first",
                repo(Some((true, true, true))),
                None,
                MergeMethod::Squash,
            ),
            (
                "then merge",
                repo(Some((false, true, true))),
                None,
                MergeMethod::Merge,
            ),
            (
                "then rebase",
                repo(Some((false, false, true))),
                None,
                MergeMethod::Rebase,
            ),
            ("unknown settings", repo(None), None, MergeMethod::Squash),
        ];
        for (name, repo, method, expected) in cases {
            assert_eq!(resolve_method(&repo, method), expected, "{name}");
        }
        assert_eq!(MergeMethod::Squash.as_str(), "squash");
        assert_eq!(
            toml::from_str::<std::collections::BTreeMap<String, MergeMethod>>("m = \"rebase\"")
                .unwrap()["m"],
            MergeMethod::Rebase
        );
    }

    #[test]
    fn squash_message_cases() {
        let cases = [
            ("Add the thing", "Body", ("Add the thing (#7)", "Body")),
            ("  Spaces  ", "", ("Spaces (#7)", "")),
            (
                "Fix",
                "Line one\r\nLine two  \r\n\r\n",
                ("Fix (#7)", "Line one\nLine two"),
            ),
            ("Keep", "a  \nb\n  c\t\n", ("Keep (#7)", "a  \nb\n  c")),
        ];
        for (title, body, (want_title, want_body)) in cases {
            let mut pr = pr(7, "", 0);
            pr.head.value.title = title.into();
            pr.head.value.body = body.into();
            assert_eq!(
                squash_message(&pr),
                (want_title.to_owned(), want_body.to_owned()),
                "{title:?}"
            );
        }
    }

    #[test]
    fn merge_blockers_cases() {
        let entry = |number: u64, draft: bool, mergeable: Mergeable, state: MergeStateStatus| {
            let mut pr = pr(number, "", 0);
            pr.head.value.is_draft = draft;
            pr.head.value.mergeable = mergeable;
            pr.head.value.merge_state = state;
            pr
        };
        let clean = |n| entry(n, false, Mergeable::Mergeable, MergeStateStatus::Clean);
        let cases = [
            ("clean", vec![clean(1), clean(2)], Blockers::default()),
            (
                "draft entry",
                vec![
                    clean(1),
                    entry(2, true, Mergeable::Mergeable, MergeStateStatus::Draft),
                ],
                Blockers {
                    hard: vec!["#2 is a draft".into()],
                    warnings: Vec::new(),
                },
            ),
            (
                "conflict below",
                vec![
                    entry(1, false, Mergeable::Conflicting, MergeStateStatus::Dirty),
                    clean(2),
                ],
                Blockers {
                    hard: vec!["#1 has conflicts".into()],
                    warnings: Vec::new(),
                },
            ),
            (
                "blocked and unknown are warnings",
                vec![
                    entry(1, false, Mergeable::Mergeable, MergeStateStatus::Blocked),
                    entry(2, false, Mergeable::Unknown, MergeStateStatus::Unknown),
                ],
                Blockers {
                    hard: Vec::new(),
                    warnings: vec![
                        "#1 is blocked by the branch rules".into(),
                        "#2 merge state is not known yet".into(),
                    ],
                },
            ),
        ];
        for (name, entries, expected) in cases {
            assert_eq!(merge_blockers(&entries), expected, "{name}");
        }
    }
}
