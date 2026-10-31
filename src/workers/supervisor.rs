//! Restarts workers that panic, with backoff.
//!
//! Each worker runs inside its own supervisor task. When the worker
//! panics, the supervisor sends `Msg::WorkerDied`, waits, and starts the
//! worker again with a clone of the context. The wait doubles after each
//! panic: 1 s, 2 s, 4 s, up to 60 s. The request receivers sit in the
//! context behind a shared lock, so a restarted worker keeps them.

use std::any::Any;
use std::time::Duration;

use futures::future::BoxFuture;
use tokio::task::JoinSet;

use crate::bus::Msg;
use crate::workers::ctx::WorkerCtx;

/// The wait before the first restart.
pub(crate) const FIRST_BACKOFF: Duration = Duration::from_secs(1);

/// The longest wait before a restart.
pub(crate) const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// A worker: its name for the status bar and the function that runs it.
#[derive(Clone, Copy)]
pub(crate) struct Worker {
    pub name: &'static str,
    pub run: fn(WorkerCtx) -> BoxFuture<'static, ()>,
}

/// Runs `worker` until it returns. After a panic it sends
/// `Msg::WorkerDied`, waits, and runs the worker again. The first wait is
/// `first`, and each next wait doubles, up to `MAX_BACKOFF`. It stops when
/// the worker returns, when the task is cancelled, or when `app` is gone.
pub(crate) async fn supervise(worker: Worker, ctx: WorkerCtx, first: Duration) {
    let mut delay = first;
    loop {
        // The worker runs in a set that this task owns, so an abort of
        // the supervisor also aborts the worker.
        let mut set = JoinSet::new();
        set.spawn((worker.run)(ctx.clone()));
        let Some(result) = set.join_next().await else {
            return;
        };
        match result {
            Ok(()) => {
                tracing::debug!(worker = worker.name, "worker stopped");
                return;
            }
            Err(err) if err.is_panic() => {
                let reason = panic_text(&err.into_panic());
                tracing::error!(worker = worker.name, %reason, ?delay, "worker panicked, restarting");
                let died = Msg::WorkerDied {
                    name: worker.name,
                    restart_in: delay,
                };
                if ctx.tx.send(died).await.is_err() {
                    return;
                }
                tokio::time::sleep(delay).await;
                delay = next_delay(delay);
            }
            Err(_) => return,
        }
    }
}

/// The wait after `delay`: double, but not more than `MAX_BACKOFF`.
fn next_delay(delay: Duration) -> Duration {
    (delay * 2).min(MAX_BACKOFF)
}

/// The message of a panic payload.
fn panic_text(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "unknown panic".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures::FutureExt;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::workers::testkit;

    static RUNS: AtomicUsize = AtomicUsize::new(0);

    /// Panics on the first run. Sends `Msg::Signal` on the next run and
    /// returns.
    async fn panics_once(ctx: WorkerCtx) {
        assert!(RUNS.fetch_add(1, Ordering::SeqCst) > 0, "boom");
        ctx.tx.send(Msg::Signal).await.unwrap();
    }

    #[tokio::test]
    async fn a_panicked_worker_restarts_after_worker_died() {
        let (ctx, mut harness) = testkit::ctx("http://127.0.0.1:1");
        let worker = Worker {
            name: "test",
            run: |ctx| panics_once(ctx).boxed(),
        };
        let first = Duration::from_millis(10);
        let supervisor = tokio::spawn(supervise(worker, ctx, first));
        assert_eq!(
            harness.recv().await,
            Msg::WorkerDied {
                name: "test",
                restart_in: first,
            }
        );
        assert_eq!(harness.recv().await, Msg::Signal);
        supervisor.await.unwrap();
        assert_eq!(RUNS.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn backoff_doubles_after_each_panic() {
        let (ctx, mut harness) = testkit::ctx("http://127.0.0.1:1");
        let worker = Worker {
            name: "always",
            run: |_| async { panic!("always") }.boxed(),
        };
        let supervisor = tokio::spawn(supervise(worker, ctx, Duration::from_millis(20)));
        let mut waits = Vec::new();
        for _ in 0..3 {
            match harness.recv().await {
                Msg::WorkerDied { restart_in, .. } => waits.push(restart_in.as_millis()),
                other => panic!("{other:?}"),
            }
        }
        supervisor.abort();
        assert_eq!(waits, [20, 40, 80]);
    }

    #[test]
    fn delays_double_up_to_the_limit() {
        let mut delay = FIRST_BACKOFF;
        let mut secs = vec![delay.as_secs()];
        for _ in 0..7 {
            delay = next_delay(delay);
            secs.push(delay.as_secs());
        }
        assert_eq!(secs, [1, 2, 4, 8, 16, 32, 60, 60]);
    }

    #[test]
    fn panic_text_reads_str_and_string() {
        assert_eq!(panic_text(&"boom"), "boom");
        assert_eq!(panic_text(&String::from("bang")), "bang");
        assert_eq!(panic_text(&1_u8), "unknown panic");
    }
}
