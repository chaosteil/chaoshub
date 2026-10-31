//! Local jj workspaces and changes.

use serde::{Deserialize, Serialize};

/// A jj workspace and its working-copy commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Workspace {
    pub name: String,
    pub change_id: String,
    pub commit_id: String,
}

/// A change in the stack of a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LocalChange {
    /// The name of the workspace whose stack holds the change.
    pub workspace: String,
    pub change_id: String,
    pub commit_id: String,
    pub bookmarks: Vec<String>,
    pub empty: bool,
    /// The change ids of the parents.
    pub parents: Vec<String>,
    /// The first line of the description.
    pub description: String,
}

/// The result of one jj scan of a repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LocalState {
    pub workspaces: Vec<Workspace>,
    pub changes: Vec<LocalChange>,
}

/// How a pull request relates to a local change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum LocalMatch {
    /// No local change matches.
    None,
    /// A local change has the same commit as the pull request head.
    Synced {
        workspace: String,
        change_id: String,
        commit_id: String,
    },
    /// A local change matches, but its commit differs from the head.
    Diverged {
        workspace: String,
        change_id: String,
        commit_id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::assert_round_trip;

    #[test]
    fn local_state_round_trip() {
        assert_round_trip(&LocalState {
            workspaces: vec![Workspace {
                name: "default".into(),
                change_id: "kxyzkxyz".into(),
                commit_id: "abc123".into(),
            }],
            changes: vec![LocalChange {
                workspace: "default".into(),
                change_id: "kxyzkxyz".into(),
                commit_id: "abc123".into(),
                bookmarks: vec!["chaosteil/kxyzkxyz".into()],
                empty: false,
                parents: vec!["zzzzzzzz".into()],
                description: "Add a thing".into(),
            }],
        });
    }

    #[test]
    fn local_match_round_trip() {
        assert_round_trip(&LocalMatch::None);
        assert_round_trip(&LocalMatch::Diverged {
            workspace: "feature".into(),
            change_id: "kxyzkxyz".into(),
            commit_id: "abc123".into(),
        });
    }
}
