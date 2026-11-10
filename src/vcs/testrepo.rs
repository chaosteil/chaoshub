//! Temporary git-only repos for the tests. The git calls of the setup
//! read no user and no system config.

use std::path::{Path, PathBuf};

/// True when git is not on the `PATH`.
pub(crate) fn git_missing() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_err()
}

/// Runs git with `args` in `dir`, without user and system config, and
/// asserts success. Gives the trimmed standard output.
pub(crate) fn git(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// The path as a string, for the git arguments of a test.
pub(crate) fn s(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// A git-only repo at `<tmp>/repo`, without `.jj`. Its origin is the bare
/// repo `<tmp>/origin.git`. The branch `main` has the commit "first", and
/// the branch `me/feature` adds the commit "second". The main worktree
/// has `main`, and the worktree `<tmp>/feature` has `me/feature`. Origin
/// has both branches.
pub(crate) fn git_only_repo(tmp: &Path) -> PathBuf {
    let origin = tmp.join("origin.git");
    let repo = tmp.join("repo");
    git(tmp, &["init", "--quiet", "--bare", s(&origin)]);
    git(tmp, &["init", "--quiet", s(&repo)]);
    git(
        &repo,
        &["commit", "--quiet", "--allow-empty", "-m", "first"],
    );
    git(&repo, &["branch", "me/feature"]);
    git(
        &repo,
        &[
            "worktree",
            "add",
            "--quiet",
            s(&tmp.join("feature")),
            "me/feature",
        ],
    );
    git(
        &tmp.join("feature"),
        &["commit", "--quiet", "--allow-empty", "-m", "second"],
    );
    git(&repo, &["remote", "add", "origin", s(&origin)]);
    git(&repo, &["push", "--quiet", "origin", "main", "me/feature"]);
    repo
}
