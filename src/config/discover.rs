//! Finds the repo of the current directory from the git origin url.
//!
//! The startup worker runs this, so no subprocess runs before the first
//! frame.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::process::Command;

use crate::config::schema::parse_repo_name;
use crate::domain::keys::RepoKey;
use crate::domain::repo::Repo;

/// The repo of `cwd`, from `git remote get-url origin`. The path is the
/// nearest directory at or above `cwd` that holds `.jj` or `.git`. `None`
/// when git fails or the url names no `owner/name` repo.
pub(crate) async fn cwd_repo(cwd: &Path) -> Option<Repo> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["remote", "get-url", "origin"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .await;
    let output = match output {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            tracing::debug!(status = %output.status, "git remote get-url origin failed");
            return None;
        }
        Err(err) => {
            tracing::debug!(%err, "cannot run git");
            return None;
        }
    };
    let url = String::from_utf8_lossy(&output.stdout);
    let key = parse_origin(url.trim())?;
    Some(Repo {
        key,
        path: Some(repo_root(cwd)),
        settings: None,
    })
}

/// Reads the `owner/name` of a remote url. It accepts the ssh forms
/// `git@host:owner/name.git` and `ssh://git@host[:port]/owner/name.git`,
/// and the `https://[user@]host/owner/name[.git]` form.
pub(crate) fn parse_origin(url: &str) -> Option<RepoKey> {
    let path = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?.1,
        None => url.split_once(':')?.1,
    };
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    parse_repo_name(path)
}

/// The nearest directory at or above `dir` that holds `.jj` or `.git`.
/// `dir` itself when no such directory exists.
pub(crate) fn repo_root(dir: &Path) -> PathBuf {
    dir.ancestors()
        .find(|candidate| candidate.join(".jj").exists() || candidate.join(".git").exists())
        .unwrap_or(dir)
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(owner: &str, name: &str) -> RepoKey {
        RepoKey {
            owner: owner.into(),
            name: name.into(),
        }
    }

    #[test]
    fn parse_origin_cases() {
        let cases = [
            "git@github.com:chaosteil/chaoshub.git",
            "git@github.com:chaosteil/chaoshub",
            "ssh://git@github.com/chaosteil/chaoshub.git",
            "ssh://git@github.com:22/chaosteil/chaoshub.git",
            "https://github.com/chaosteil/chaoshub.git",
            "https://github.com/chaosteil/chaoshub",
            "https://github.com/chaosteil/chaoshub/",
            "https://user:token@github.com/chaosteil/chaoshub.git",
        ];
        for url in cases {
            assert_eq!(
                parse_origin(url),
                Some(key("chaosteil", "chaoshub")),
                "{url}"
            );
        }
    }

    #[test]
    fn parse_origin_rejects_other_forms() {
        let cases = [
            "",
            "/local/path/repo",
            "https://github.com/chaosteil",
            "https://github.com",
            "git@github.com:a/b/c.git",
        ];
        for url in cases {
            assert_eq!(parse_origin(url), None, "{url}");
        }
    }

    #[test]
    fn repo_root_finds_the_nearest_marker() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("a/b");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(repo_root(&sub), sub);
        std::fs::create_dir(dir.path().join(".jj")).unwrap();
        assert_eq!(repo_root(&sub), dir.path());
    }

    fn git(dir: &Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    #[tokio::test]
    async fn cwd_repo_reads_the_origin() {
        let dir = tempfile::tempdir().unwrap();
        if !git(dir.path(), &["init", "-q"]) {
            eprintln!("git is missing, skipped");
            return;
        }
        let sub = dir.path().join("src");
        std::fs::create_dir(&sub).unwrap();
        assert_eq!(cwd_repo(&sub).await, None);
        assert!(git(
            dir.path(),
            &[
                "remote",
                "add",
                "origin",
                "git@github.com:chaosteil/chaoshub.git"
            ]
        ));
        assert_eq!(
            cwd_repo(&sub).await,
            Some(Repo {
                key: key("chaosteil", "chaoshub"),
                path: Some(dir.path().to_path_buf()),
                settings: None,
            })
        );
    }
}
