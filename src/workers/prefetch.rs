//! The prefetch worker: fetches the git refs of pull requests into the
//! local repo, so the fetch of codeview transfers nothing.
//!
//! It acts on the first `Interest` and on each change, after 250 ms
//! without a further change.
//! It fetches the head of the selected pull request first, then the
//! heads of the visible ones. Before the first head of a base ref, it
//! fetches that base ref once per sync period (`refresh.full_secs`). It
//! fetches the base ref again when a sync reports a new base commit
//! (`baseRefOid`) for a pull request, so a review of codeview finds the
//! new base. Each git process runs under the lock of its repo. A head is
//! fetched again only when its oid changes. The worker sends `PrefetchDone` per pull
//! request, and `PrefetchBatch` at the start of a batch and after each
//! pull request of it. A batch is the pull requests of one `Interest`
//! that need a fetch. It does nothing when `prefetch.enabled` is false.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures::FutureExt;

use crate::bus::{Msg, PrRef};
use crate::domain::keys::{PrKey, RepoKey};
use crate::vcs::git;
use crate::workers::ctx::WorkerCtx;
use crate::workers::settle;
use crate::workers::supervisor::Worker;

/// The prefetch worker.
pub(crate) const WORKER: Worker = Worker {
    name: "prefetch",
    run: |ctx| run(ctx).boxed(),
};

/// The quiet time after an `Interest` change before the worker acts.
const DEBOUNCE: Duration = Duration::from_millis(250);

/// The refs that the worker fetched.
#[derive(Debug, Default)]
struct Fetched {
    /// The base refs of this sync period, per repo, with the base commit
    /// of the pull request that caused the fetch. A new base commit
    /// fetches the base ref again.
    bases: HashSet<(RepoKey, String, String)>,
    /// The head oid of the last fetch of each pull request, and whether
    /// that fetch succeeded.
    heads: HashMap<PrKey, (String, bool)>,
}

impl Fetched {
    /// Starts a new sync period: every base ref and every failed head is
    /// fetched again.
    fn new_period(&mut self) {
        self.bases.clear();
        self.heads.retain(|_, (_, ok)| *ok);
    }

    /// The pull requests of `order` that need a fetch of the base ref or
    /// of the head, in order. A base ref with one base commit counts once.
    fn pending(&self, order: impl Iterator<Item = PrRef>) -> Vec<PrRef> {
        let mut bases = self.bases.clone();
        order
            .filter(|pr| {
                let base = bases.insert(base_key(pr));
                let known = self.heads.get(&pr.key).map(|(oid, _)| oid);
                base || known != Some(&pr.head_oid)
            })
            .collect()
    }
}

/// The key of the base ref of `pr` in `Fetched::bases`.
fn base_key(pr: &PrRef) -> (RepoKey, String, String) {
    (
        pr.key.repo.clone(),
        pr.base_ref.clone(),
        pr.base_oid.clone(),
    )
}

/// Serves `Interest` changes until `app` is gone.
async fn run(ctx: WorkerCtx) {
    if !ctx.config.prefetch.enabled {
        tracing::info!("prefetch is off");
        return;
    }
    let mut interest = ctx.interest.clone();
    // The first interest is the value of the watch, not a change. The
    // pinned mode can keep it for its whole life, so it is fetched too.
    interest.mark_changed();
    let period = Duration::from_secs(ctx.config.refresh.full_secs.max(1));
    let mut period_start = Instant::now();
    let mut fetched = Fetched::default();
    // True while `app` shows a batch that did not end.
    let mut open_batch = false;
    loop {
        if interest.changed().await.is_err() || !settle(&mut interest, DEBOUNCE).await {
            return;
        }
        if period_start.elapsed() >= period {
            fetched.new_period();
            period_start = Instant::now();
        }
        let current = interest.borrow_and_update().clone();
        let paths: HashMap<RepoKey, PathBuf> = ctx
            .session
            .borrow()
            .repos
            .iter()
            .filter_map(|repo| Some((repo.key.clone(), repo.path.clone()?)))
            .collect();
        let selected = current.selected.clone();
        let order = selected.iter().cloned().chain(
            current
                .visible
                .into_iter()
                .filter(|pr| Some(pr) != selected.as_ref()),
        );
        let batch = fetched.pending(order.filter(|pr| paths.contains_key(&pr.key.repo)));
        let total = batch.len();
        if (total > 0 || open_batch) && !send_batch(&ctx, 0, total).await {
            return;
        }
        open_batch = total > 0;
        for (index, pr) in batch.iter().enumerate() {
            // A new selection starts over with the new order.
            if interest.has_changed().unwrap_or(true) {
                break;
            }
            let Some(path) = paths.get(&pr.key.repo) else {
                continue;
            };
            if !prefetch(&ctx, &mut fetched, path, pr).await {
                return;
            }
            if !send_batch(&ctx, index + 1, total).await {
                return;
            }
            open_batch = index + 1 < total;
        }
    }
}

/// Sends the progress of the batch. False when `app` is gone.
async fn send_batch(ctx: &WorkerCtx, done: usize, total: usize) -> bool {
    ctx.tx
        .send(Msg::PrefetchBatch { done, total })
        .await
        .is_ok()
}

/// Fetches the base ref and the head of `pr` when they are not fetched
/// yet, and sends `PrefetchDone`. False when `app` is gone.
async fn prefetch(ctx: &WorkerCtx, fetched: &mut Fetched, path: &Path, pr: &PrRef) -> bool {
    let mut ran = false;
    let mut errors = Vec::new();
    if fetched.bases.insert(base_key(pr)) {
        ran = true;
        if let Err(err) = git::prefetch_base(&ctx.locks, path, &pr.base_ref).await {
            tracing::warn!(key = ?pr.key, base = %pr.base_ref, %err, "base prefetch failed");
            errors.push(format!("base {}: {err}", pr.base_ref));
        }
    }
    let known = fetched.heads.get(&pr.key).map(|(oid, _)| oid);
    if known != Some(&pr.head_oid) {
        ran = true;
        let head = git::prefetch_head(&ctx.locks, path, pr.key.number).await;
        fetched
            .heads
            .insert(pr.key.clone(), (pr.head_oid.clone(), head.is_ok()));
        if let Err(err) = head {
            tracing::warn!(key = ?pr.key, %err, "head prefetch failed");
            errors.push(format!("head: {err}"));
        }
    }
    if !ran {
        return true;
    }
    let result = if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    };
    ctx.tx
        .send(Msg::PrefetchDone(pr.key.clone(), result))
        .await
        .is_ok()
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::bus::Session;
    use crate::config::Config;
    use crate::domain::repo::Repo;
    use crate::workers::testkit;

    /// True when the `git` binary is not on the path.
    fn git_missing() -> bool {
        std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
    }

    /// Runs git with `args` and returns its trimmed output. It asserts
    /// success.
    fn git_setup(args: &[&str]) -> String {
        let output = std::process::Command::new("git")
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    fn s(path: &Path) -> &str {
        path.to_str().unwrap()
    }

    /// A repo whose origin is a local bare repo with `main` and the pull
    /// request head 7. Returns the repo path and the commit.
    fn origin_repo(tmp: &Path) -> (PathBuf, String) {
        let origin = tmp.join("origin.git");
        let work = tmp.join("work");
        let repo = tmp.join("repo");
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
        let commit = git_setup(&["-C", s(&work), "rev-parse", "HEAD"]);
        git_setup(&[
            "-C",
            s(&work),
            "push",
            "--quiet",
            s(&origin),
            "HEAD:refs/heads/main",
            "HEAD:refs/pull/7/head",
        ]);
        git_setup(&["init", "--quiet", s(&repo)]);
        git_setup(&["-C", s(&repo), "remote", "add", "origin", s(&origin)]);
        (repo, commit)
    }

    #[tokio::test]
    async fn fetches_base_and_head_once_from_a_local_origin() {
        if git_missing() {
            eprintln!("skipped: git is missing");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let (path, commit) = origin_repo(tmp.path());
        let (ctx, mut harness) = testkit::ctx("http://127.0.0.1:1");
        harness
            .session
            .send(Session {
                login: Some("chaosteil".into()),
                repos: vec![Repo {
                    key: testkit::repo(),
                    path: Some(path.clone()),
                    settings: None,
                }],
            })
            .unwrap();
        let worker = tokio::spawn(run(ctx));

        let seven = testkit::pr_ref(7, "PR_7", &commit);
        let interest = testkit::interest(Some(seven.clone()), vec![seven.clone()]);
        harness.interest.send(interest.clone()).unwrap();
        assert_eq!(
            harness.recv().await,
            Msg::PrefetchBatch { done: 0, total: 1 }
        );
        assert_eq!(
            harness.recv().await,
            Msg::PrefetchDone(seven.key.clone(), Ok(()))
        );
        assert_eq!(
            harness.recv().await,
            Msg::PrefetchBatch { done: 1, total: 1 }
        );
        for name in ["refs/chaoshub/base/main", "refs/chaoshub/pr/7/head"] {
            assert_eq!(
                git_setup(&["-C", s(&path), "rev-parse", "--verify", name]),
                commit
            );
        }

        harness.interest.send(interest).unwrap();
        assert!(harness.quiet_for(Duration::from_millis(600)).await);

        let eight = testkit::pr_ref(8, "PR_8", "missing");
        harness
            .interest
            .send(testkit::interest(None, vec![seven, eight.clone()]))
            .unwrap();
        assert_eq!(
            harness.recv().await,
            Msg::PrefetchBatch { done: 0, total: 1 }
        );
        match harness.recv().await {
            Msg::PrefetchDone(key, Err(err)) => {
                assert_eq!(key, eight.key);
                assert!(err.starts_with("head: "), "{err}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            harness.recv().await,
            Msg::PrefetchBatch { done: 1, total: 1 }
        );
        worker.abort();
    }

    #[tokio::test]
    async fn a_new_base_commit_fetches_the_base_ref_again() {
        if git_missing() {
            eprintln!("skipped: git is missing");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let (path, commit) = origin_repo(tmp.path());
        let (ctx, mut harness) = testkit::ctx("http://127.0.0.1:1");
        harness
            .session
            .send(Session {
                login: None,
                repos: vec![Repo {
                    key: testkit::repo(),
                    path: Some(path.clone()),
                    settings: None,
                }],
            })
            .unwrap();
        let worker = tokio::spawn(run(ctx));

        let mut seven = testkit::pr_ref(7, "PR_7", &commit);
        seven.base_oid = commit.clone();
        harness
            .interest
            .send(testkit::interest(Some(seven.clone()), Vec::new()))
            .unwrap();
        for _ in 0..3 {
            harness.recv().await;
        }

        // A push to the base branch on origin, then a sync that reports
        // the new base commit.
        let work = tmp.path().join("work");
        let origin = tmp.path().join("origin.git");
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
            "second",
        ]);
        let base = git_setup(&["-C", s(&work), "rev-parse", "HEAD"]);
        git_setup(&[
            "-C",
            s(&work),
            "push",
            "--quiet",
            s(&origin),
            "HEAD:refs/heads/main",
        ]);
        seven.base_oid = base.clone();
        harness
            .interest
            .send(testkit::interest(Some(seven.clone()), Vec::new()))
            .unwrap();
        assert_eq!(
            harness.recv().await,
            Msg::PrefetchBatch { done: 0, total: 1 }
        );
        assert_eq!(
            harness.recv().await,
            Msg::PrefetchDone(seven.key.clone(), Ok(()))
        );
        assert_eq!(
            harness.recv().await,
            Msg::PrefetchBatch { done: 1, total: 1 }
        );
        assert_eq!(
            git_setup(&[
                "-C",
                s(&path),
                "rev-parse",
                "--verify",
                "refs/chaoshub/base/main"
            ]),
            base
        );
        assert_eq!(
            git_setup(&[
                "-C",
                s(&path),
                "rev-parse",
                "--verify",
                "refs/chaoshub/pr/7/head"
            ]),
            commit
        );
        worker.abort();
    }

    #[tokio::test]
    async fn the_first_interest_is_fetched_without_a_change() {
        if git_missing() {
            eprintln!("skipped: git is missing");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let (path, commit) = origin_repo(tmp.path());
        let (mut ctx, mut harness) = testkit::ctx("http://127.0.0.1:1");
        harness
            .session
            .send(Session {
                login: None,
                repos: vec![Repo {
                    key: testkit::repo(),
                    path: Some(path),
                    settings: None,
                }],
            })
            .unwrap();
        let seven = testkit::pr_ref(7, "PR_7", &commit);
        harness
            .interest
            .send(testkit::interest(Some(seven), Vec::new()))
            .unwrap();
        // `run` makes the watch with the first interest: it is not a
        // change.
        ctx.interest.borrow_and_update();
        let worker = tokio::spawn(run(ctx));
        assert_eq!(
            harness.recv().await,
            Msg::PrefetchBatch { done: 0, total: 1 }
        );
        worker.abort();
    }

    #[tokio::test]
    async fn a_disabled_prefetch_does_nothing() {
        let mut config = Config::default();
        config.prefetch.enabled = false;
        let (ctx, mut harness) =
            testkit::ctx_with("http://127.0.0.1:1", config, testkit::memory_store());
        tokio::time::timeout(Duration::from_secs(1), run(ctx))
            .await
            .unwrap();
        assert!(harness.quiet_for(Duration::from_millis(50)).await);
    }

    #[test]
    fn pending_skips_fetched_refs_and_counts_a_base_once() {
        let mut fetched = Fetched::default();
        let known = testkit::pr_ref(1, "PR_1", "a");
        let moved = testkit::pr_ref(2, "PR_2", "b");
        let fresh = testkit::pr_ref(3, "PR_3", "c");
        fetched.bases.insert(base_key(&known));
        fetched.heads.insert(known.key.clone(), ("a".into(), true));
        fetched
            .heads
            .insert(moved.key.clone(), ("old".into(), true));
        let order = [known.clone(), moved.clone(), fresh.clone()];
        assert_eq!(fetched.pending(order.into_iter()), [moved, fresh]);
        let mut other_base = known.clone();
        other_base.base_ref = "release".into();
        assert_eq!(
            fetched.pending([known, other_base.clone()].into_iter()),
            [other_base]
        );
    }

    #[test]
    fn pending_fetches_the_base_again_for_a_new_base_commit() {
        let mut fetched = Fetched::default();
        let mut known = testkit::pr_ref(1, "PR_1", "a");
        known.base_oid = "base1".into();
        fetched.bases.insert(base_key(&known));
        fetched.heads.insert(known.key.clone(), ("a".into(), true));
        assert_eq!(fetched.pending([known.clone()].into_iter()), []);

        let mut moved = known.clone();
        moved.base_oid = "base2".into();
        let mut other = testkit::pr_ref(2, "PR_2", "b");
        other.base_oid = "base2".into();
        fetched.heads.insert(other.key.clone(), ("b".into(), true));
        // The second pull request with the same new base commit counts
        // once.
        assert_eq!(fetched.pending([moved.clone(), other].into_iter()), [moved]);
    }

    #[test]
    fn a_new_period_keeps_only_good_heads() {
        let mut fetched = Fetched::default();
        fetched
            .bases
            .insert((testkit::repo(), "main".into(), String::new()));
        let good = testkit::pr_ref(1, "PR_1", "a");
        let bad = testkit::pr_ref(2, "PR_2", "b");
        fetched.heads.insert(good.key.clone(), ("a".into(), true));
        fetched.heads.insert(bad.key.clone(), ("b".into(), false));
        fetched.new_period();
        assert!(fetched.bases.is_empty());
        assert_eq!(fetched.heads.keys().collect::<Vec<_>>(), [&good.key]);
    }
}
