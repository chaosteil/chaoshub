//! Parsers for the output of the jj templates.

use super::error::VcsError;
use crate::domain::local::{LocalChange, Workspace};

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
    }
}
