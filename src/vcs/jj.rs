//! Reads jj workspaces and stacks, and runs `jj git fetch`.
//!
//! Every jj call runs with `--ignore-working-copy`, so it never snapshots
//! a working copy.

use std::path::{Path, PathBuf};

use tokio::process::Command;

use super::error::VcsError;
use super::lock::RepoLocks;
use super::{parse, run};
use crate::domain::local::{LocalChange, Workspace};

/// The workspace list template: name, change id and commit id.
const WORKSPACE_TEMPLATE: &str =
    r#"name ++ "\x1f" ++ target.change_id() ++ "\x1f" ++ target.commit_id() ++ "\n""#;

/// The stack log template: change id, commit id, bookmarks, empty flag,
/// parents and the first line of the description, which is last.
const STACK_TEMPLATE: &str = r#"change_id ++ "\x1f" ++ commit_id ++ "\x1f" ++ bookmarks.map(|b| b.name()).join(",") ++ "\x1f" ++ if(empty,"1","0") ++ "\x1f" ++ parents.map(|p| p.change_id()).join(",") ++ "\x1f" ++ description.first_line() ++ "\n""#;

/// Lists the workspaces of the repo at `repo_path`.
pub(crate) async fn list_workspaces(repo_path: &Path) -> Result<Vec<Workspace>, VcsError> {
    parse::workspaces(&run(list_workspaces_command(repo_path)).await?)
}

/// Reads the stack of the workspace `ws_name`: its changes from the fork
/// point with trunk up to its working-copy commit, top first. The repo at
/// `repo_path` reads it with the `<name>@` revset.
pub(crate) async fn read_stack(
    repo_path: &Path,
    ws_name: &str,
) -> Result<Vec<LocalChange>, VcsError> {
    parse::stack(ws_name, &run(read_stack_command(repo_path, ws_name)).await?)
}

/// Runs `jj git fetch` in the repo at `repo_path`, under the lock of the
/// repo.
pub(crate) async fn git_fetch(locks: &RepoLocks, repo_path: &Path) -> Result<(), VcsError> {
    let _guard = locks.lock(repo_path).await;
    run(git_fetch_command(repo_path)).await.map(drop)
}

/// The default root of the workspace directories: `~/code/workspaces`.
/// `None` when the home directory is unknown.
pub(crate) fn default_workspaces_root() -> Option<PathBuf> {
    etcetera::home_dir()
        .ok()
        .map(|home| home.join("code").join("workspaces"))
}

/// The directory of the workspace `ws_name`: `repo_path` for `default`,
/// else `<root>/<folder>/<ws_name>`. `<folder>` is the folder name of
/// `repo_path`, not the GitHub name of the repo: `~/code/chaoshub` gives
/// `chaoshub`, also when the repo is `chaosteil/hub`. `jj workspace list`
/// prints no paths, so this is a convention. The directory can be
/// missing. A `repo_path` without a folder name, for example `/`, gives
/// `<root>/<ws_name>`.
pub(crate) fn workspace_path(repo_path: &Path, ws_name: &str, root: &Path) -> PathBuf {
    if ws_name == "default" {
        repo_path.to_path_buf()
    } else {
        root.join(repo_path.file_name().unwrap_or_default())
            .join(ws_name)
    }
}

/// A jj command on `repo_path` with the flags of every call: no snapshot,
/// no pager, no color.
fn jj(repo_path: &Path) -> Command {
    let mut cmd = Command::new("jj");
    cmd.arg("-R")
        .arg(repo_path)
        .args(["--ignore-working-copy", "--no-pager", "--color=never"]);
    cmd
}

fn list_workspaces_command(repo_path: &Path) -> Command {
    let mut cmd = jj(repo_path);
    cmd.args(["workspace", "list", "-T", WORKSPACE_TEMPLATE]);
    cmd
}

/// The revset of the stack of `ws_name`, without the changes in trunk.
fn stack_revset(ws_name: &str) -> String {
    format!("(fork_point(trunk()|{ws_name}@)::{ws_name}@) ~ ::trunk()")
}

fn read_stack_command(repo_path: &Path, ws_name: &str) -> Command {
    let mut cmd = jj(repo_path);
    cmd.args(["log", "--no-graph", "-r"])
        .arg(stack_revset(ws_name))
        .args(["-T", STACK_TEMPLATE]);
    cmd
}

fn git_fetch_command(repo_path: &Path) -> Command {
    let mut cmd = jj(repo_path);
    cmd.args(["git", "fetch"]);
    cmd
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::vcs::tests::args;

    const READ_FLAGS: [&str; 5] = [
        "-R",
        "/repo",
        "--ignore-working-copy",
        "--no-pager",
        "--color=never",
    ];

    fn expected(rest: &[&str]) -> Vec<String> {
        READ_FLAGS
            .iter()
            .chain(rest)
            .map(|arg| (*arg).to_owned())
            .collect()
    }

    #[test]
    fn workspace_list_argv() {
        let cmd = list_workspaces_command(Path::new("/repo"));
        assert_eq!(cmd.as_std().get_program(), "jj");
        assert_eq!(
            args(&cmd),
            expected(&[
                "workspace",
                "list",
                "-T",
                r#"name ++ "\x1f" ++ target.change_id() ++ "\x1f" ++ target.commit_id() ++ "\n""#,
            ])
        );
    }

    #[test]
    fn read_stack_argv() {
        let cmd = read_stack_command(Path::new("/repo"), "feature");
        assert_eq!(
            args(&cmd),
            expected(&[
                "log",
                "--no-graph",
                "-r",
                "(fork_point(trunk()|feature@)::feature@) ~ ::trunk()",
                "-T",
                STACK_TEMPLATE,
            ])
        );
    }

    #[test]
    fn git_fetch_argv() {
        let cmd = git_fetch_command(Path::new("/repo"));
        assert_eq!(args(&cmd), expected(&["git", "fetch"]));
    }

    #[test]
    fn workspace_paths() {
        let repo = Path::new("/home/me/code/chaoshub");
        let root = Path::new("/home/me/code/workspaces");
        assert_eq!(workspace_path(repo, "default", root), repo);
        let cases = [
            (
                "/home/me/code/chaoshub",
                "/home/me/code/workspaces/chaoshub/b-vcs-launch",
            ),
            (
                "/home/me/code/chaoshub/",
                "/home/me/code/workspaces/chaoshub/b-vcs-launch",
            ),
            (
                "/home/me/src/hub-main",
                "/home/me/code/workspaces/hub-main/b-vcs-launch",
            ),
            ("/", "/home/me/code/workspaces/b-vcs-launch"),
        ];
        for (repo, expected) in cases {
            assert_eq!(
                workspace_path(Path::new(repo), "b-vcs-launch", root),
                Path::new(expected),
                "{repo}"
            );
        }
    }

    /// True when the `jj` binary is not on the path.
    fn jj_missing() -> bool {
        std::process::Command::new("jj")
            .arg("--version")
            .output()
            .is_err()
    }

    /// Runs a jj command that changes the test repo and asserts success.
    fn jj_setup<S: AsRef<OsStr>>(args: impl IntoIterator<Item = S>) {
        let args: Vec<S> = args.into_iter().collect();
        let output = std::process::Command::new("jj")
            .args(&args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "jj: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn reads_a_temporary_repo() {
        if jj_missing() {
            eprintln!("skipped: jj is missing");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let second = tmp.path().join("second");
        let in_repo = |args: &[&str]| {
            jj_setup(
                [OsStr::new("-R"), repo.as_os_str()]
                    .into_iter()
                    .chain(args.iter().map(OsStr::new)),
            );
        };
        jj_setup([OsStr::new("git"), OsStr::new("init"), repo.as_os_str()]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        in_repo(&["describe", "-m", "first change"]);
        in_repo(&["new", "-m", "second change"]);
        in_repo(&["bookmark", "create", "feature", "-r", "@-"]);
        in_repo(&[
            "workspace",
            "add",
            "--name",
            "second",
            second.to_str().unwrap(),
        ]);

        let workspaces = list_workspaces(&repo).await.unwrap();
        let names: Vec<&str> = workspaces.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["default", "second"]);

        let stack = read_stack(&repo, "default").await.unwrap();
        assert_eq!(stack.len(), 2);
        assert_eq!(stack[0].workspace, "default");
        assert_eq!(stack[0].description, "second change");
        assert!(stack[0].empty);
        assert_eq!(stack[0].change_id, workspaces[0].change_id);
        assert_eq!(stack[0].parents, [stack[1].change_id.clone()]);
        assert_eq!(stack[1].description, "first change");
        assert_eq!(stack[1].bookmarks, ["feature"]);
        assert!(!stack[1].empty);

        let second_stack = read_stack(&repo, "second").await.unwrap();
        assert_eq!(second_stack.len(), 2);
        assert_eq!(second_stack[1].change_id, stack[1].change_id);
    }
}
