//! Builds the `Target` of a row: the pull request with its loaded stack
//! entries, its local match and its repo, or a local-only change and its
//! repo.

use crate::app::models::RowItem;
use crate::app::state::AppData;
use crate::bus::Target;
use crate::domain::keys::{PrKey, RepoKey};
use crate::domain::local::LocalMatch;
use crate::domain::matching::match_prs;
use crate::domain::pr::PullRequest;
use crate::domain::repo::Repo;

/// The target of a loaded pull request. `None` when it is not loaded.
pub(crate) fn pr_target(data: &AppData, key: &PrKey) -> Option<Target> {
    let pr = data.prs.get(key)?;
    let stack = match &pr.head.value.stack {
        Some(stack) => {
            let mut entries: Vec<PullRequest> = data
                .prs
                .values()
                .filter(|other| {
                    other
                        .head
                        .value
                        .stack
                        .as_ref()
                        .is_some_and(|s| s.id == stack.id)
                })
                .cloned()
                .collect();
            entries.sort_by_key(|entry| entry.head.value.stack.as_ref().map(|s| s.position));
            entries
        }
        None => Vec::new(),
    };
    let local = data
        .local
        .get(&key.repo)
        .and_then(|scan| {
            match_prs(&[pr], &scan.value.changes)
                .into_iter()
                .next()
                .map(|(_, m)| m)
        })
        .unwrap_or(LocalMatch::None);
    let repo = data.repos.get(&key.repo).cloned().unwrap_or_else(|| Repo {
        key: key.repo.clone(),
        path: None,
        settings: Some(pr.head.value.repo_settings.clone()),
    });
    Some(Target::Pr {
        pr: pr.clone(),
        stack,
        local,
        repo,
    })
}

/// The target of a local-only change of the last jj scan of `repo`.
/// `None` when the scan has no such change.
pub(crate) fn local_target(data: &AppData, repo: &RepoKey, change_id: &str) -> Option<Target> {
    let change = data
        .local
        .get(repo)?
        .value
        .changes
        .iter()
        .find(|change| change.change_id == change_id)?
        .clone();
    let repo = data.repos.get(repo).cloned().unwrap_or_else(|| Repo {
        key: repo.clone(),
        path: None,
        settings: None,
    });
    Some(Target::Local { change, repo })
}

/// The target of a row. `None` for a header, a separator, or an item
/// that is not loaded.
pub(crate) fn row_target(data: &AppData, item: &RowItem) -> Option<Target> {
    match item {
        RowItem::Pr(pr) => pr_target(data, &pr.key),
        RowItem::Local { repo, change_id } => local_target(data, repo, change_id),
        RowItem::StackHeader { .. } | RowItem::Separator => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::AppState;
    use crate::app::testdata;
    use crate::bus::Size;

    #[test]
    fn a_stacked_pr_carries_its_entries_and_its_local_match() {
        let mut state = AppState::new(Size {
            width: 100,
            height: 30,
        });
        state.load(testdata::mine_snapshot(), Vec::new());
        let key = testdata::pr_key("chaoshub", 103);
        let Some(Target::Pr {
            pr,
            stack,
            local,
            repo,
        }) = pr_target(&state.data, &key)
        else {
            panic!("no target");
        };
        assert_eq!(pr.key, key);
        let positions: Vec<u64> = stack.iter().map(|entry| entry.key.number).collect();
        assert_eq!(positions, [101, 102, 103], "position 1 first");
        assert!(
            matches!(local, LocalMatch::Synced { change_id, .. } if change_id == "wqnvzxopwqnv")
        );
        assert_eq!(repo.key, key.repo);
        assert_eq!(
            pr_target(&state.data, &testdata::pr_key("chaoshub", 1)),
            None
        );
    }

    #[test]
    fn a_local_row_targets_its_change_and_its_repo() {
        let mut state = AppState::new(Size {
            width: 100,
            height: 30,
        });
        state.load(testdata::mine_snapshot(), Vec::new());
        let (repo, scan) = testdata::mine_local();
        let change = scan.value.changes[0].clone();
        let item = RowItem::Local {
            repo: repo.clone(),
            change_id: change.change_id.clone(),
        };
        let Some(Target::Local {
            change: got,
            repo: got_repo,
        }) = row_target(&state.data, &item)
        else {
            panic!("no target");
        };
        assert_eq!(got, change);
        assert_eq!(got_repo.key, repo);
        let missing = RowItem::Local {
            repo,
            change_id: "nosuchchange".into(),
        };
        assert_eq!(row_target(&state.data, &missing), None);
        let header = RowItem::StackHeader {
            stack_id: "S".into(),
        };
        assert_eq!(row_target(&state.data, &header), None);
    }

    #[test]
    fn a_single_pr_has_no_stack() {
        let mut state = AppState::new(Size {
            width: 100,
            height: 30,
        });
        state.load(testdata::mine_snapshot(), Vec::new());
        let target = pr_target(&state.data, &testdata::pr_key("chaoshub", 99));
        assert!(matches!(
            target,
            Some(Target::Pr { stack, local: LocalMatch::None, .. }) if stack.is_empty()
        ));
    }
}
