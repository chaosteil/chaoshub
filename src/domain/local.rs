//! Local jj workspaces and changes, and local git worktrees and branches.

use serde::{Deserialize, Serialize};

/// The tool that reads a local repo.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Vcs {
    /// jj reads the repo.
    #[default]
    Jj,
    /// git reads the repo.
    Git,
}

/// A jj workspace and its working-copy commit. In a git repo, a worktree:
/// the change id is the branch name of the worktree, and empty for a
/// detached worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Workspace {
    pub name: String,
    pub change_id: String,
    pub commit_id: String,
}

/// A change in the stack of a workspace. In a git repo, a local branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LocalChange {
    /// The name of the workspace whose stack holds the change. In a git
    /// repo, the name of the worktree that has the branch, and empty when
    /// no worktree has it.
    pub workspace: String,
    /// The jj change id. In a git repo, the branch name.
    pub change_id: String,
    /// The commit id. In a git repo, the tip of the branch.
    pub commit_id: String,
    /// The bookmarks of the change. In a git repo, the branch name.
    pub bookmarks: Vec<String>,
    pub empty: bool,
    /// The change ids of the parents.
    pub parents: Vec<String>,
    /// The first line of the description.
    pub description: String,
    /// The tool that read the change. A cached scan without this field is
    /// a jj scan.
    #[serde(default)]
    pub vcs: Vcs,
}

/// The result of one jj or git scan of a repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LocalState {
    pub workspaces: Vec<Workspace>,
    pub changes: Vec<LocalChange>,
}

/// How a pull request relates to a local change. In a git repo, the
/// change id is the branch name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum LocalMatch {
    /// No local change matches.
    None,
    /// A local change has the same commit as the pull request head.
    Synced {
        workspace: String,
        change_id: String,
        commit_id: String,
        #[serde(default)]
        vcs: Vcs,
    },
    /// A local change matches, but its commit differs from the head.
    Diverged {
        workspace: String,
        change_id: String,
        commit_id: String,
        #[serde(default)]
        vcs: Vcs,
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
                vcs: Vcs::Jj,
            }],
        });
    }

    #[test]
    fn a_cached_change_without_a_tool_is_a_jj_change() {
        let json = r#"{"workspace":"default","change_id":"kxyz","commit_id":"abc","bookmarks":[],"empty":false,"parents":[],"description":""}"#;
        let change: LocalChange = serde_json::from_str(json).unwrap();
        assert_eq!(change.vcs, Vcs::Jj);
    }

    #[test]
    fn local_match_round_trip() {
        assert_round_trip(&LocalMatch::None);
        assert_round_trip(&LocalMatch::Diverged {
            workspace: "feature".into(),
            change_id: "kxyzkxyz".into(),
            commit_id: "abc123".into(),
            vcs: Vcs::Jj,
        });
        assert_round_trip(&LocalMatch::Synced {
            workspace: "default".into(),
            change_id: "me/feature".into(),
            commit_id: "abc123".into(),
            vcs: Vcs::Git,
        });
    }
}
