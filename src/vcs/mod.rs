//! Adapter for jj workspaces and stacks, git worktrees and branches, and
//! git ref prefetch.
//!
//! Each repo uses jj or git. A repo with a `.jj` directory uses jj when
//! jj is on the `PATH`. Every other repo uses git. A colocated repo has
//! `.jj` and `.git`, so it uses jj. A jj repo without `.git` is not
//! supported.
#![allow(dead_code, reason = "the workers use the adapter in a later phase")]

pub(crate) mod error;
pub(crate) mod git;
pub(crate) mod jj;
pub(crate) mod lock;
mod parse;

use std::ffi::OsStr;
use std::path::Path;

use tokio::process::Command;

use self::error::VcsError;
use crate::domain::local::Vcs;

/// The tool that reads a repo, and the reason for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tool {
    /// The repo has a `.jj` directory, and jj is on the `PATH`.
    Jj,
    /// The repo has no `.jj` directory.
    Git,
    /// The repo has a `.jj` directory, but jj is not on the `PATH`.
    GitWithoutJj,
}

impl Tool {
    /// The tool as a `Vcs`.
    pub(crate) fn vcs(self) -> Vcs {
        match self {
            Self::Jj => Vcs::Jj,
            Self::Git | Self::GitWithoutJj => Vcs::Git,
        }
    }
}

/// The tool of the repo at `repo_path`. `jj_found` is true when jj is on
/// the `PATH`. A jj repo without `.git` gives `VcsError::PlainJj`.
pub(crate) fn detect(repo_path: &Path, jj_found: bool) -> Result<Tool, VcsError> {
    if !repo_path.join(".jj").is_dir() {
        return Ok(Tool::Git);
    }
    if is_plain_jj(repo_path) {
        return Err(VcsError::PlainJj {
            path: repo_path.to_path_buf(),
        });
    }
    Ok(if jj_found {
        Tool::Jj
    } else {
        Tool::GitWithoutJj
    })
}

/// True for a jj repo that is not colocated: `.jj/repo` is a directory,
/// and `.git` does not exist. A second jj workspace has a `.jj/repo`
/// file, so it is not a plain jj repo.
pub(crate) fn is_plain_jj(repo_path: &Path) -> bool {
    repo_path.join(".jj").join("repo").is_dir() && !repo_path.join(".git").exists()
}

/// True when jj is on the `PATH`.
pub(crate) fn jj_on_path() -> bool {
    on_path("jj", std::env::var_os("PATH").as_deref())
}

/// True when a directory of the path list `path_var` holds the file
/// `program`.
pub(crate) fn on_path(program: &str, path_var: Option<&OsStr>) -> bool {
    path_var.is_some_and(|var| std::env::split_paths(var).any(|dir| dir.join(program).is_file()))
}

/// Runs `cmd` and returns its standard output. An exit status other than
/// zero gives an error with the standard error text.
async fn run(mut cmd: Command) -> Result<String, VcsError> {
    let program = cmd.as_std().get_program().to_string_lossy().into_owned();
    let output = cmd.output().await.map_err(|source| VcsError::Spawn {
        program: program.clone(),
        source,
    })?;
    if !output.status.success() {
        return Err(VcsError::Failed {
            program,
            status: output.status,
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use pretty_assertions::assert_eq;
    use tokio::process::Command;

    use super::*;

    /// The arguments of `cmd` as strings, without the program.
    pub(super) fn args(cmd: &Command) -> Vec<String> {
        cmd.as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn detect_cases() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        assert_eq!(detect(repo, true).unwrap(), Tool::Git, "no .jj");
        std::fs::create_dir(repo.join(".git")).unwrap();
        assert_eq!(detect(repo, true).unwrap(), Tool::Git, "a git repo");
        std::fs::create_dir_all(repo.join(".jj/repo")).unwrap();
        assert_eq!(detect(repo, true).unwrap(), Tool::Jj, "a colocated repo");
        assert_eq!(
            detect(repo, false).unwrap(),
            Tool::GitWithoutJj,
            "a colocated repo without jj"
        );
        std::fs::remove_dir(repo.join(".git")).unwrap();
        assert!(matches!(
            detect(repo, true),
            Err(VcsError::PlainJj { path }) if path == repo
        ));
    }

    #[test]
    fn a_second_jj_workspace_is_not_a_plain_jj_repo() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".jj")).unwrap();
        std::fs::write(dir.path().join(".jj/repo"), "../main/.jj/repo").unwrap();
        assert!(!is_plain_jj(dir.path()));
        assert_eq!(detect(dir.path(), true).unwrap(), Tool::Jj);
    }

    #[test]
    fn on_path_finds_files_in_the_path_list() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        std::fs::write(bin.join("git"), "").unwrap();
        let var: OsString = std::env::join_paths([dir.path(), bin.as_path()]).unwrap();
        assert!(on_path("git", Some(&var)));
        assert!(!on_path("jj", Some(&var)));
        assert!(!on_path("bin", Some(&var)), "a directory is no program");
        assert!(!on_path("git", None));
    }
}
