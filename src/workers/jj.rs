//! The jj worker: scans the local workspaces of a repo, with jj or git.
//!
//! It serves `JjReq::Scan(repo)` and runs its own timer: every
//! `refresh.jj_secs` (30 s by default) while `Interest.wants_local` is
//! true, it scans every repo with a path. A scan first finds the tool of
//! the repo (see `vcs::detect`). A jj scan lists the workspaces and reads
//! the stack of each one. A git scan lists the worktrees and reads the
//! local branches. The result is one `Observed<LocalState>` per repo, so
//! an older slow scan never replaces a newer one. The worker sends
//! `JjScanned` first and then writes the scan as one store row.
//!
//! A repo that uses git is logged once, not on every scan. A jj repo
//! without `.git` sends one `SyncFailed` with the source `scan`.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures::FutureExt;
use tokio::time::MissedTickBehavior;

use crate::bus::{JjReq, Msg, SyncError};
use crate::domain::keys::RepoKey;
use crate::domain::local::LocalState;
use crate::domain::observed::Observed;
use crate::vcs::error::VcsError;
use crate::vcs::{self, Tool, git, jj};
use crate::workers::ctx::WorkerCtx;
use crate::workers::supervisor::Worker;
use crate::workers::wait_for_login;

/// The jj worker.
pub(crate) const WORKER: Worker = Worker {
    name: "jj",
    run: |ctx| run(ctx).boxed(),
};

/// Waits for the login, then serves requests and timer ticks until `app`
/// is gone.
async fn run(ctx: WorkerCtx) {
    let mut session = ctx.session.clone();
    if wait_for_login(&mut session).await.is_none() {
        return;
    }
    let jj_rx = Arc::clone(&ctx.jj_rx);
    let mut requests = jj_rx.lock().await;
    let period = Duration::from_secs(ctx.config.refresh.jj_secs.max(1));
    let mut ticker = tokio::time::interval(period);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut noted = HashSet::new();
    // The first tick fires at once. `app` asks for the first scans.
    ticker.tick().await;
    loop {
        tokio::select! {
            request = requests.recv() => match request {
                Some(JjReq::Scan(repo)) => {
                    if !scan_repo(&ctx, &repo, &mut noted).await {
                        return;
                    }
                }
                None => return,
            },
            _ = ticker.tick() => {
                if !ctx.interest.borrow().wants_local {
                    continue;
                }
                let repos: Vec<RepoKey> = ctx
                    .session
                    .borrow()
                    .repos
                    .iter()
                    .filter(|repo| repo.path.is_some())
                    .map(|repo| repo.key.clone())
                    .collect();
                for repo in repos {
                    if !scan_repo(&ctx, &repo, &mut noted).await {
                        return;
                    }
                }
            }
        }
    }
}

/// Scans `repo` when the session has a path for it, sends `JjScanned`
/// and writes the store row. `noted` holds the repos whose tool is
/// already logged. False when `app` is gone.
async fn scan_repo(ctx: &WorkerCtx, repo: &RepoKey, noted: &mut HashSet<RepoKey>) -> bool {
    let path = ctx
        .session
        .borrow()
        .repos
        .iter()
        .find(|known| known.key == *repo)
        .and_then(|known| known.path.clone());
    let Some(path) = path else {
        tracing::debug!(?repo, "jj scan skipped: the repo has no path");
        return true;
    };
    let tool = match vcs::detect(&path, vcs::jj_on_path()) {
        Ok(tool) => tool,
        Err(err) => {
            if !noted.insert(repo.clone()) {
                return true;
            }
            tracing::warn!(?repo, %err, "cannot scan the repo");
            let failed = Msg::SyncFailed {
                source: "scan",
                scope: None,
                err: SyncError::Other(err.to_string()),
            };
            return ctx.tx.send(failed).await.is_ok();
        }
    };
    if tool != Tool::Jj && noted.insert(repo.clone()) {
        match tool {
            Tool::GitWithoutJj => {
                tracing::info!(?repo, "jj is not on the PATH, the scan uses git");
            }
            Tool::Git | Tool::Jj => {
                tracing::info!(?repo, "the repo has no .jj, the scan uses git");
            }
        }
    }
    let observed_at = Utc::now();
    let value = match scan(&path, tool).await {
        Ok(value) => value,
        Err(err) => {
            tracing::warn!(?repo, %err, "local scan failed");
            return true;
        }
    };
    let state = Observed { value, observed_at };
    let msg = Msg::JjScanned {
        repo: repo.clone(),
        state: state.clone(),
    };
    if ctx.tx.send(msg).await.is_err() {
        return false;
    }
    if let Err(err) = ctx.store.put_local(repo.clone(), state).await {
        tracing::warn!(?repo, %err, "cannot cache the jj scan");
    }
    true
}

/// The local state of the repo at `path`, read with `tool`.
async fn scan(path: &Path, tool: Tool) -> Result<LocalState, VcsError> {
    match tool {
        Tool::Jj => scan_jj(path).await,
        Tool::Git | Tool::GitWithoutJj => scan_git(path).await,
    }
}

/// The workspaces of the jj repo at `path` and the stack of each one. A
/// workspace whose stack does not read is left out.
async fn scan_jj(path: &Path) -> Result<LocalState, VcsError> {
    let workspaces = jj::list_workspaces(path).await?;
    let mut changes = Vec::new();
    for workspace in &workspaces {
        match jj::read_stack(path, &workspace.name).await {
            Ok(mut stack) => changes.append(&mut stack),
            Err(err) => {
                tracing::warn!(workspace = %workspace.name, %err, "cannot read the jj stack");
            }
        }
    }
    Ok(LocalState {
        workspaces,
        changes,
    })
}

/// The worktrees and the local branches of the git repo at `path`.
async fn scan_git(path: &Path) -> Result<LocalState, VcsError> {
    let workspaces = git::list_worktrees(path).await?;
    let changes = git::read_branches(path, &workspaces).await?;
    Ok(LocalState {
        workspaces,
        changes,
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::PathBuf;

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::bus::Session;
    use crate::config::Config;
    use crate::domain::repo::Repo;
    use crate::store::Store;
    use crate::workers::testkit;

    /// True when the `jj` binary is not on the path.
    fn jj_missing() -> bool {
        std::process::Command::new("jj")
            .arg("--version")
            .output()
            .is_err()
    }

    /// Runs jj with `args` and asserts success.
    fn jj_setup<S: AsRef<OsStr>>(args: &[S]) {
        let output = std::process::Command::new("jj")
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "jj: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// A jj repo with two changes in `default` and a second workspace.
    fn temp_repo(tmp: &Path) -> PathBuf {
        let repo = tmp.join("repo");
        let second = tmp.join("second");
        let in_repo = |args: &[&str]| {
            let mut all = vec![OsStr::new("-R"), repo.as_os_str()];
            all.extend(args.iter().map(OsStr::new));
            jj_setup(&all);
        };
        jj_setup(&[OsStr::new("git"), OsStr::new("init"), repo.as_os_str()]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        in_repo(&["describe", "-m", "first change"]);
        in_repo(&["new", "-m", "second change"]);
        in_repo(&[
            "workspace",
            "add",
            "--name",
            "second",
            second.to_str().unwrap(),
        ]);
        repo
    }

    fn names(state: &LocalState) -> Vec<&str> {
        state.workspaces.iter().map(|ws| ws.name.as_str()).collect()
    }

    #[tokio::test]
    async fn scans_a_temporary_repo_on_request_and_on_the_timer() {
        if jj_missing() {
            eprintln!("skipped: jj is missing");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let path = temp_repo(tmp.path());
        let db = tmp.path().join("cache.db");
        let store = Store::open(&db).unwrap().1;
        let mut config = Config::default();
        config.refresh.jj_secs = 1;
        let (ctx, mut harness) = testkit::ctx_with("http://127.0.0.1:1", config, store);
        harness
            .session
            .send(Session {
                login: Some("chaosteil".into()),
                repos: vec![Repo {
                    key: testkit::repo(),
                    path: Some(path),
                    settings: None,
                }],
            })
            .unwrap();
        let worker = tokio::spawn(run(ctx));

        harness.jj.send(JjReq::Scan(testkit::repo())).await.unwrap();
        let Msg::JjScanned { repo, state } = harness.recv().await else {
            panic!("expected JjScanned");
        };
        assert_eq!(repo, testkit::repo());
        assert_eq!(names(&state.value), ["default", "second"]);
        let descriptions: Vec<(&str, &str)> = state
            .value
            .changes
            .iter()
            .map(|change| (change.workspace.as_str(), change.description.as_str()))
            .collect();
        assert_eq!(
            descriptions,
            [
                ("default", "second change"),
                ("default", "first change"),
                ("second", ""),
                ("second", "first change"),
            ]
        );

        let mut cached = Vec::new();
        for _ in 0..50 {
            cached = Store::open(&db).unwrap().0.local;
            if !cached.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(cached, [(testkit::repo(), state)]);

        assert!(harness.quiet_for(Duration::from_millis(1500)).await);
        harness
            .interest
            .send_modify(|interest| interest.wants_local = true);
        assert!(matches!(harness.recv().await, Msg::JjScanned { .. }));
        worker.abort();
    }

    #[tokio::test]
    async fn a_repo_without_a_path_is_skipped() {
        let (ctx, mut harness) = testkit::ctx("http://127.0.0.1:1");
        harness
            .session
            .send(Session {
                login: Some("chaosteil".into()),
                repos: vec![Repo {
                    key: testkit::repo(),
                    path: None,
                    settings: None,
                }],
            })
            .unwrap();
        assert!(scan_repo(&ctx, &testkit::repo(), &mut HashSet::new()).await);
        assert!(harness.quiet_for(Duration::from_millis(50)).await);
    }

    #[tokio::test]
    async fn no_requests_before_the_login() {
        let (ctx, mut harness) = testkit::ctx("http://127.0.0.1:1");
        let worker = tokio::spawn(run(ctx));
        harness.jj.send(JjReq::Scan(testkit::repo())).await.unwrap();
        assert!(harness.quiet_for(Duration::from_millis(100)).await);
        assert!(!worker.is_finished());
        worker.abort();
    }
}
