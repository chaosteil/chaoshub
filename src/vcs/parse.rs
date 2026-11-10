//! Parsers for the output of the jj templates and of the git commands.

use std::path::Path;

use super::error::VcsError;
use crate::domain::local::{LocalChange, Vcs, Workspace};

/// The name of the main git worktree, the same as the default jj
/// workspace.
const MAIN_WORKTREE: &str = "default";

/// The field separator of the templates.
const SEP: char = '\x1f';

/// Parses the output of the workspace list template: name, change id and
/// commit id per line.
pub(super) fn workspaces(out: &str) -> Result<Vec<Workspace>, VcsError> {
    out.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut fields = line.splitn(3, SEP);
            match (fields.next(), fields.next(), fields.next()) {
                (Some(name), Some(change_id), Some(commit_id)) => Ok(Workspace {
                    name: name.to_owned(),
                    change_id: change_id.to_owned(),
                    commit_id: commit_id.to_owned(),
                }),
                _ => Err(parse_error("workspace list", line)),
            }
        })
        .collect()
}

/// Parses the output of the stack log template of the workspace
/// `workspace`: change id, commit id, bookmarks, empty flag, parents and
/// the first line of the description per line. The description is the
/// last field, so it can hold any text.
pub(super) fn stack(workspace: &str, out: &str) -> Result<Vec<LocalChange>, VcsError> {
    out.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let fields: Vec<&str> = line.splitn(6, SEP).collect();
            let [change_id, commit_id, bookmarks, empty, parents, description] = fields[..] else {
                return Err(parse_error("stack log", line));
            };
            let empty = match empty {
                "1" => true,
                "0" => false,
                _ => return Err(parse_error("stack log", line)),
            };
            Ok(LocalChange {
                workspace: workspace.to_owned(),
                change_id: change_id.to_owned(),
                commit_id: commit_id.to_owned(),
                bookmarks: list(bookmarks),
                empty,
                parents: list(parents),
                description: description.to_owned(),
                vcs: Vcs::Jj,
            })
        })
        .collect()
}

/// Parses the output of `git worktree list --porcelain`: one block of
/// lines per worktree, with an empty line after each block. The first
/// worktree is the main worktree, `default`. Every other worktree has the
/// name of its directory. A detached worktree has an empty change id.
pub(super) fn worktrees(out: &str) -> Result<Vec<Workspace>, VcsError> {
    out.split("\n\n")
        .filter(|block| !block.trim().is_empty())
        .enumerate()
        .map(|(index, block)| {
            let mut path = None;
            let mut head = "";
            let mut branch = "";
            for line in block.lines() {
                if let Some(rest) = line.strip_prefix("worktree ") {
                    path = Some(rest);
                } else if let Some(rest) = line.strip_prefix("HEAD ") {
                    head = rest;
                } else if let Some(rest) = line.strip_prefix("branch ") {
                    branch = rest.strip_prefix("refs/heads/").unwrap_or(rest);
                }
            }
            let Some(path) = path else {
                return Err(parse_error("worktree list", block));
            };
            let name = if index == 0 {
                MAIN_WORKTREE.to_owned()
            } else {
                Path::new(path).file_name().map_or_else(
                    || path.to_owned(),
                    |name| name.to_string_lossy().into_owned(),
                )
            };
            Ok(Workspace {
                name,
                change_id: branch.to_owned(),
                commit_id: head.to_owned(),
            })
        })
        .collect()
}

/// Parses the output of the branch format of `git for-each-ref`: branch
/// name, tip commit and subject per line. A branch gets the name of the
/// worktree in `worktrees` that has it, else an empty workspace.
pub(super) fn branches(out: &str, worktrees: &[Workspace]) -> Result<Vec<LocalChange>, VcsError> {
    out.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut fields = line.splitn(3, SEP);
            let (Some(name), Some(commit_id), Some(subject)) =
                (fields.next(), fields.next(), fields.next())
            else {
                return Err(parse_error("branch list", line));
            };
            let workspace = worktrees
                .iter()
                .find(|worktree| worktree.change_id == name)
                .map(|worktree| worktree.name.clone())
                .unwrap_or_default();
            Ok(LocalChange {
                workspace,
                change_id: name.to_owned(),
                commit_id: commit_id.to_owned(),
                bookmarks: vec![name.to_owned()],
                empty: false,
                parents: Vec::new(),
                description: subject.to_owned(),
                vcs: Vcs::Git,
            })
        })
        .collect()
}

/// Splits a comma list. An empty field is an empty list.
fn list(field: &str) -> Vec<String> {
    field
        .split(',')
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn parse_error(what: &'static str, line: &str) -> VcsError {
    VcsError::Parse {
        what,
        line: line.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    const WORKSPACE_LIST: &str = include_str!("testdata/workspace_list.txt");
    const STACK_LOG: &str = include_str!("testdata/stack_log.txt");

    #[test]
    fn workspaces_from_captured_output() {
        let workspaces = workspaces(WORKSPACE_LIST).unwrap();
        assert_eq!(workspaces.len(), 6);
        assert_eq!(
            workspaces[4],
            Workspace {
                name: "b-vcs-launch".into(),
                change_id: "xqykylwxzlknqyuknwmvopyrvqpxlvpn".into(),
                commit_id: "56e995e6cacb05b46b73480567a84ab3922130e8".into(),
            }
        );
        assert_eq!(workspaces[5].name, "default");
    }

    #[test]
    fn stack_from_captured_output() {
        let changes = stack("b-vcs-launch", STACK_LOG).unwrap();
        assert_eq!(changes.len(), 12);
        assert_eq!(
            changes[0],
            LocalChange {
                workspace: "b-vcs-launch".into(),
                change_id: "xqykylwxzlknqyuknwmvopyrvqpxlvpn".into(),
                commit_id: "56e995e6cacb05b46b73480567a84ab3922130e8".into(),
                bookmarks: vec![],
                empty: true,
                parents: vec!["yytlukzkkywoxkvpstplyzzmxsylynmr".into()],
                description: "feat(vcs): jj workspace list, stack read and git fetch".into(),
                vcs: Vcs::Jj,
            }
        );
        assert!(!changes[1].empty);
        assert_eq!(changes[11].parents, ["zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"]);
    }

    #[test]
    fn stack_with_bookmarks_merge_parents_and_separator_in_description() {
        let out = "kkkk\x1fc1\x1fchaosteil/kkkk,feature\x1f0\x1fpppp,qqqq\x1fa\x1fb\n";
        let changes = stack("default", out).unwrap();
        assert_eq!(changes[0].bookmarks, ["chaosteil/kkkk", "feature"]);
        assert_eq!(changes[0].parents, ["pppp", "qqqq"]);
        assert_eq!(changes[0].description, "a\x1fb");
    }

    #[test]
    fn stack_with_empty_description() {
        let changes = stack("default", "kkkk\x1fc1\x1f\x1f1\x1fpppp\x1f\n").unwrap();
        assert_eq!(changes[0].description, "");
        assert!(changes[0].empty);
    }

    #[test]
    fn malformed_lines_are_errors() {
        assert!(matches!(
            workspaces("default\x1fkkkk\n"),
            Err(VcsError::Parse { .. })
        ));
        assert!(matches!(
            stack("default", "kkkk\x1fc1\x1f\x1f1\n"),
            Err(VcsError::Parse { .. })
        ));
        assert!(matches!(
            stack("default", "kkkk\x1fc1\x1f\x1fyes\x1f\x1f\n"),
            Err(VcsError::Parse { .. })
        ));
    }

    #[test]
    fn empty_output_is_empty() {
        assert_eq!(workspaces("").unwrap(), []);
        assert_eq!(stack("default", "\n").unwrap(), []);
        assert_eq!(worktrees("").unwrap(), []);
        assert_eq!(branches("\n", &[]).unwrap(), []);
    }

    const WORKTREE_LIST: &str = "\
worktree /home/me/code/chaoshub
HEAD 1111111111111111111111111111111111111111
branch refs/heads/main

worktree /home/me/code/workspaces/chaoshub/feature
HEAD 2222222222222222222222222222222222222222
branch refs/heads/me/feature
locked

worktree /home/me/code/workspaces/chaoshub/spike
HEAD 3333333333333333333333333333333333333333
detached

";

    #[test]
    fn worktrees_from_porcelain_output() {
        let workspace = |name: &str, branch: &str, head: &str| Workspace {
            name: name.into(),
            change_id: branch.into(),
            commit_id: head.into(),
        };
        assert_eq!(
            worktrees(WORKTREE_LIST).unwrap(),
            [
                workspace("default", "main", &"1".repeat(40)),
                workspace("feature", "me/feature", &"2".repeat(40)),
                workspace("spike", "", &"3".repeat(40)),
            ]
        );
    }

    #[test]
    fn the_main_worktree_of_a_bare_repo_is_default() {
        let list = worktrees("worktree /srv/repo.git\nbare\n\n").unwrap();
        assert_eq!(list[0].name, "default");
        assert_eq!(list[0].change_id, "");
    }

    #[test]
    fn branches_get_the_name_of_their_worktree() {
        let out =
            "main\x1f1111\x1fdocs: a readme\nme/feature\x1f2222\x1ffeat: a\x1fb\nold\x1f4444\x1f\n";
        let changes = branches(out, &worktrees(WORKTREE_LIST).unwrap()).unwrap();
        assert_eq!(
            changes[1],
            LocalChange {
                workspace: "feature".into(),
                change_id: "me/feature".into(),
                commit_id: "2222".into(),
                bookmarks: vec!["me/feature".into()],
                empty: false,
                parents: Vec::new(),
                description: "feat: a\x1fb".into(),
                vcs: Vcs::Git,
            }
        );
        assert_eq!(changes[0].workspace, "default");
        assert_eq!(changes[2].workspace, "", "no worktree has the branch");
        assert_eq!(changes[2].description, "");
    }

    #[test]
    fn malformed_git_lines_are_errors() {
        assert!(matches!(
            worktrees("HEAD 1111\n\n"),
            Err(VcsError::Parse { .. })
        ));
        assert!(matches!(
            branches("main\x1f1111\n", &[]),
            Err(VcsError::Parse { .. })
        ));
    }
}
