//! The fuzzy filter of a tab, on a nucleo matcher.
//!
//! A tab keeps one `Filter` in its `TabState.ext`. `update` asks the tab
//! to score again after the filter text or the data changed; the filter
//! skips the work when neither moved. The tab model then reads the
//! matches without a mutable borrow.

use std::collections::HashSet;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use crate::domain::keys::{PrKey, RepoKey};
use crate::domain::local::LocalChange;
use crate::domain::pr::PullRequest;

/// What the filter matches.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum ItemKey {
    Pr(PrKey),
    Local { repo: RepoKey, change_id: String },
}

/// The fuzzy filter state of one tab.
#[derive(Debug, Default)]
pub(crate) struct Filter {
    /// Made on the first non-empty filter text, then reused.
    matcher: Option<Matcher>,
    /// The filter text and the data revision of the last scoring.
    scored: Option<(String, u64)>,
    /// The items that match. `None` when the filter text is empty.
    matches: Option<HashSet<ItemKey>>,
}

impl Filter {
    /// Scores the candidates again when `text` or `revision` changed
    /// since the last call. True when it scored.
    pub(crate) fn rescore(
        &mut self,
        text: &str,
        revision: u64,
        candidates: impl IntoIterator<Item = (ItemKey, String)>,
    ) -> bool {
        if self
            .scored
            .as_ref()
            .is_some_and(|(t, r)| t == text && *r == revision)
        {
            return false;
        }
        self.scored = Some((text.to_owned(), revision));
        if text.trim().is_empty() {
            self.matches = None;
            return true;
        }
        let matcher = self
            .matcher
            .get_or_insert_with(|| Matcher::new(Config::DEFAULT));
        let pattern = Pattern::parse(text, CaseMatching::Smart, Normalization::Smart);
        let mut buf = Vec::new();
        self.matches = Some(
            candidates
                .into_iter()
                .filter(|(_, haystack)| {
                    pattern
                        .score(Utf32Str::new(haystack, &mut buf), matcher)
                        .is_some()
                })
                .map(|(key, _)| key)
                .collect(),
        );
        true
    }

    /// True when the item passes the filter.
    pub(crate) fn keeps(&self, key: &ItemKey) -> bool {
        self.matches
            .as_ref()
            .is_none_or(|matches| matches.contains(key))
    }
}

/// The filter candidate of a pull request: its title, number, author and
/// branch.
pub(crate) fn pr_candidate(pr: &PullRequest) -> (ItemKey, String) {
    let head = &pr.head.value;
    (
        ItemKey::Pr(pr.key.clone()),
        format!(
            "{} #{} {} {}",
            head.title, pr.key.number, head.author, head.head_ref
        ),
    )
}

/// The filter candidate of a local change: its description, change id
/// and workspace.
pub(crate) fn local_candidate(repo: &RepoKey, change: &LocalChange) -> (ItemKey, String) {
    (
        ItemKey::Local {
            repo: repo.clone(),
            change_id: change.change_id.clone(),
        },
        format!(
            "{} {} {}",
            change.description, change.change_id, change.workspace
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testdata;

    fn candidates() -> Vec<(ItemKey, String)> {
        let mut alice = testdata::pr("chaoshub", 2, "Fix the export");
        alice.head.value.author = "alice".into();
        vec![
            pr_candidate(&testdata::pr("chaoshub", 1, "Add the mine tab")),
            pr_candidate(&alice),
            pr_candidate(&testdata::pr("chaoshub", 31, "Docs")),
        ]
    }

    fn kept(filter: &Filter) -> Vec<u64> {
        let mut numbers: Vec<u64> = candidates()
            .into_iter()
            .filter(|(key, _)| filter.keeps(key))
            .filter_map(|(key, _)| match key {
                ItemKey::Pr(pr) => Some(pr.number),
                ItemKey::Local { .. } => None,
            })
            .collect();
        numbers.sort_unstable();
        numbers
    }

    #[test]
    fn filter_cases() {
        let cases = [
            ("", vec![1, 2, 31], "empty"),
            ("  ", vec![1, 2, 31], "blank"),
            ("mine", vec![1], "title"),
            (
                "Mine",
                vec![],
                "smart case: an upper case letter matches case",
            ),
            ("alice", vec![2], "author"),
            ("#31", vec![31], "number"),
            ("chaosteil/2", vec![2], "branch"),
            ("fx exp", vec![2], "fuzzy words"),
            ("zzz", vec![], "no match"),
        ];
        for (text, expected, name) in cases {
            let mut filter = Filter::default();
            filter.rescore(text, 0, candidates());
            assert_eq!(kept(&filter), expected, "{name}: {text:?}");
        }
    }

    #[test]
    fn rescore_skips_when_nothing_moved() {
        let mut filter = Filter::default();
        assert!(filter.rescore("mine", 1, candidates()));
        assert!(!filter.rescore("mine", 1, candidates()));
        assert!(filter.rescore("mine", 2, candidates()), "the data moved");
        assert!(filter.rescore("tab", 2, candidates()), "the text moved");
        assert!(filter.rescore("", 2, candidates()));
        assert_eq!(kept(&filter), [1, 2, 31]);
    }

    #[test]
    fn a_local_change_matches_its_description() {
        let repo = testdata::repo_key("chaoshub");
        let change = LocalChange {
            workspace: "layout".into(),
            change_id: "zzmqxkrt".into(),
            commit_id: "9a8b7c6d".into(),
            bookmarks: Vec::new(),
            empty: false,
            parents: Vec::new(),
            description: "wip: try the new layout".into(),
        };
        let mut filter = Filter::default();
        filter.rescore("wip", 0, [local_candidate(&repo, &change)]);
        assert!(filter.keeps(&ItemKey::Local {
            repo,
            change_id: "zzmqxkrt".into(),
        }));
    }
}
