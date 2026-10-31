//! Prefetches git refs and reads the origin url.
//!
//! The prefetch puts the objects of a pull request into the local repo
//! before a review, so the fetch of codeview transfers nothing. The refs
//! go under `refs/chaoshub/`, which jj does not import.

use std::path::Path;

use tokio::process::Command;

use super::error::VcsError;
use super::lock::RepoLocks;
use super::run;
use crate::domain::keys::RepoKey;

/// Fetches the branch `base_ref` of origin into
/// `refs/chaoshub/base/<base_ref>`, under the lock of the repo.
pub(crate) async fn prefetch_base(
    locks: &RepoLocks,
    repo_path: &Path,
    base_ref: &str,
) -> Result<(), VcsError> {
    let _guard = locks.lock(repo_path).await;
    run(prefetch_base_command(repo_path, base_ref))
        .await
        .map(drop)
}

/// Fetches the head of pull request `number` from origin into
/// `refs/chaoshub/pr/<number>/head`, under the lock of the repo.
pub(crate) async fn prefetch_head(
    locks: &RepoLocks,
    repo_path: &Path,
    number: u64,
) -> Result<(), VcsError> {
    let _guard = locks.lock(repo_path).await;
    run(prefetch_head_command(repo_path, number))
        .await
        .map(drop)
}

/// The GitHub repo of the origin remote of the repo at `path`. `None`
/// when git fails or the url has no `owner/name` path.
pub(crate) async fn origin_url(path: &Path) -> Option<RepoKey> {
    let mut cmd = git(path);
    cmd.args(["remote", "get-url", "origin"]);
    parse_origin(&run(cmd).await.ok()?)
}

/// Reads `owner/name` from an ssh or https url, for example
/// `git@github.com:owner/name.git`, `ssh://git@github.com/owner/name` or
/// `https://github.com/owner/name.git`.
fn parse_origin(url: &str) -> Option<RepoKey> {
    let url = url.trim();
    let path = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?.1,
        None => url.split_once(':')?.1,
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(name), None) if !owner.is_empty() && !name.is_empty() => Some(RepoKey {
            owner: owner.to_owned(),
            name: name.to_owned(),
        }),
        _ => None,
    }
}

/// A git command in `path`. Git never prompts for credentials, because a
/// prompt would draw over the TUI.
fn git(path: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(path).env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

/// `git fetch` of one refspec from origin, without tags and without a
/// `FETCH_HEAD` write.
fn fetch_command(repo_path: &Path, refspec: String) -> Command {
    let mut cmd = git(repo_path);
    cmd.args([
        "fetch",
        "--no-tags",
        "--force",
        "--quiet",
        "--no-write-fetch-head",
        "origin",
    ])
    .arg(refspec);
    cmd
}

fn prefetch_base_command(repo_path: &Path, base_ref: &str) -> Command {
    fetch_command(
        repo_path,
        format!("+refs/heads/{base_ref}:refs/chaoshub/base/{base_ref}"),
    )
}

fn prefetch_head_command(repo_path: &Path, number: u64) -> Command {
    fetch_command(
        repo_path,
        format!("+refs/pull/{number}/head:refs/chaoshub/pr/{number}/head"),
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::vcs::tests::args;

    const FETCH_FLAGS: [&str; 9] = [
        "-C",
        "/repo",
        "fetch",
        "--no-tags",
        "--force",
        "--quiet",
        "--no-write-fetch-head",
        "origin",
        "",
    ];

    fn expected(refspec: &str) -> Vec<String> {
        let mut argv: Vec<String> = FETCH_FLAGS.iter().map(|arg| (*arg).to_owned()).collect();
        refspec.clone_into(&mut argv[8]);
        argv
    }

    #[test]
    fn prefetch_base_argv() {
        let cmd = prefetch_base_command(Path::new("/repo"), "main");
        assert_eq!(cmd.as_std().get_program(), "git");
        assert_eq!(
            args(&cmd),
            expected("+refs/heads/main:refs/chaoshub/base/main")
        );
    }

    #[test]
    fn prefetch_head_argv() {
        let cmd = prefetch_head_command(Path::new("/repo"), 42);
        assert_eq!(
            args(&cmd),
            expected("+refs/pull/42/head:refs/chaoshub/pr/42/head")
        );
    }

    #[test]
    fn git_never_prompts() {
        let cmd = prefetch_head_command(Path::new("/repo"), 1);
        let envs: Vec<_> = cmd.as_std().get_envs().collect();
        assert_eq!(envs, [("GIT_TERMINAL_PROMPT".as_ref(), Some("0".as_ref()))]);
    }

    #[test]
    fn origin_from_ssh_and_https_urls() {
        let key = Some(RepoKey {
            owner: "chaosteil".into(),
            name: "chaoshub".into(),
        });
        for url in [
            "git@github.com:chaosteil/chaoshub.git\n",
            "git@github.com:chaosteil/chaoshub",
            "ssh://git@github.com/chaosteil/chaoshub.git",
            "https://github.com/chaosteil/chaoshub.git",
            "https://github.com/chaosteil/chaoshub",
            "https://github.com/chaosteil/chaoshub/",
            "https://user@github.com/chaosteil/chaoshub.git",
        ] {
            assert_eq!(parse_origin(url), key, "{url}");
        }
    }

    #[test]
    fn origin_without_owner_and_name() {
        for url in [
            "",
            "/home/me/origin.git",
            "https://github.com/chaosteil",
            "https://github.com/a/b/c",
            "git@github.com:/chaoshub",
        ] {
            assert_eq!(parse_origin(url), None, "{url}");
        }
    }

    /// True when the `git` binary is not on the path.
    fn git_missing() -> bool {
        std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
    }

    /// Runs a git command and asserts success.
    fn git_setup(args: &[&str]) {
        let output = std::process::Command::new("git")
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// The path as a string, for the git arguments of a test.
    fn s(path: &Path) -> &str {
        path.to_str().unwrap()
    }

    #[tokio::test]
    async fn origin_url_of_a_temporary_repo() {
        if git_missing() {
            eprintln!("skipped: git is missing");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        git_setup(&["init", "--quiet", s(&repo)]);
        assert_eq!(origin_url(&repo).await, None);
        git_setup(&[
            "-C",
            s(&repo),
            "remote",
            "add",
            "origin",
            "git@github.com:chaosteil/chaoshub.git",
        ]);
        assert_eq!(
            origin_url(&repo).await,
            Some(RepoKey {
                owner: "chaosteil".into(),
                name: "chaoshub".into(),
            })
        );
    }

    /// The time that the upload pack of the test origin sleeps.
    const UPLOAD_DELAY: Duration = Duration::from_millis(300);

    /// A repo whose origin is a local bare repo with the pull request
    /// heads 1 and 2. Every fetch from it takes at least `UPLOAD_DELAY`.
    fn slow_origin_repo(tmp: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let origin = tmp.join("origin.git");
        let work = tmp.join("work");
        let repo = tmp.join("repo");
        let script = tmp.join("slow-upload-pack");
        git_setup(&["init", "--quiet", "--bare", s(&origin)]);
        git_setup(&["init", "--quiet", s(&work)]);
        git_setup(&[
            "-C",
            s(&work),
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "first",
        ]);
        git_setup(&[
            "-C",
            s(&work),
            "push",
            "--quiet",
            s(&origin),
            "HEAD:refs/heads/main",
            "HEAD:refs/pull/1/head",
            "HEAD:refs/pull/2/head",
        ]);
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nsleep {}\nexec git upload-pack \"$@\"\n",
                UPLOAD_DELAY.as_secs_f64()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        git_setup(&["init", "--quiet", s(&repo)]);
        git_setup(&["-C", s(&repo), "remote", "add", "origin", s(&origin)]);
        git_setup(&[
            "-C",
            s(&repo),
            "config",
            "remote.origin.uploadpack",
            s(&script),
        ]);
        repo
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn two_prefetches_in_one_repo_run_in_sequence() {
        if git_missing() {
            eprintln!("skipped: git is missing");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let repo = slow_origin_repo(tmp.path());
        let locks = RepoLocks::default();

        let start = Instant::now();
        let (one, two) = tokio::join!(
            prefetch_head(&locks, &repo, 1),
            prefetch_head(&locks, &repo, 2)
        );
        let elapsed = start.elapsed();
        one.unwrap();
        two.unwrap();
        assert!(
            elapsed >= 2 * UPLOAD_DELAY,
            "the prefetches overlapped: {elapsed:?}"
        );

        prefetch_base(&locks, &repo, "main").await.unwrap();
        for name in [
            "refs/chaoshub/pr/1/head",
            "refs/chaoshub/pr/2/head",
            "refs/chaoshub/base/main",
        ] {
            git_setup(&["-C", s(&repo), "rev-parse", "--verify", "--quiet", name]);
        }
    }
}
