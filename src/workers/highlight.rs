//! The highlight worker: gives the code lines of a patch their syntax
//! colors, off the UI thread.
//!
//! It serves `HighlightReq`. It keeps the waiting jobs in a queue and
//! runs one job at a time on a blocking thread. Thus a large patch never
//! stops the UI, and the worker uses at most one core. Before each job,
//! it reads all the waiting requests: a cancel drops the job of its
//! file, and a job for a new head of a pull request drops the jobs for
//! the old heads. The result goes to `app` as `Msg::Highlighted`.
//!
//! The syntax set loads when the worker starts, on a blocking thread.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use futures::FutureExt;

use crate::bus::{HighlightJob, HighlightReq, Highlighted, Msg};
use crate::highlight;
use crate::workers::ctx::WorkerCtx;
use crate::workers::supervisor::Worker;

/// The highlight worker.
pub(crate) const WORKER: Worker = Worker {
    name: "highlight",
    run: |ctx| run(ctx).boxed(),
};

/// Serves the requests until `app` is gone.
async fn run(ctx: WorkerCtx) {
    let highlight_rx = Arc::clone(&ctx.highlight_rx);
    let mut requests = highlight_rx.lock().await;
    if let Err(err) = tokio::task::spawn_blocking(highlight::warm).await {
        tracing::warn!(%err, "cannot load the syntax set");
    }
    let mut queue = Queue::default();
    loop {
        if queue.is_empty() {
            let Some(request) = requests.recv().await else {
                return;
            };
            queue.push(request);
        }
        while let Ok(request) = requests.try_recv() {
            queue.push(request);
        }
        let Some(job) = queue.pop() else {
            continue;
        };
        let done = Msg::Highlighted(run_job(job).await);
        if ctx.tx.send(done).await.is_err() {
            return;
        }
    }
}

/// Highlights the patch of `job` on a blocking thread. A panic of the
/// highlight gives a result without syntax colors.
async fn run_job(job: HighlightJob) -> Highlighted {
    let HighlightJob {
        key,
        head,
        path,
        patch,
    } = job;
    let started = Instant::now();
    let file = path.clone();
    let lines = match tokio::task::spawn_blocking(move || highlight::patch(&file, &patch)).await {
        Ok(lines) => lines.map(Arc::new),
        Err(err) => {
            tracing::warn!(%err, path, "the highlight failed");
            None
        }
    };
    tracing::debug!(
        path,
        ms = started.elapsed().as_millis(),
        colored = lines.is_some(),
        "highlight done"
    );
    Highlighted {
        key,
        head,
        path,
        lines,
    }
}

/// The waiting jobs, oldest first.
#[derive(Debug, Default)]
struct Queue {
    jobs: VecDeque<HighlightJob>,
}

impl Queue {
    fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Applies one request. A new job replaces the waiting job of its
    /// file, and drops the waiting jobs of its pull request at another
    /// head. A cancel drops the waiting job of its file.
    fn push(&mut self, request: HighlightReq) {
        match request {
            HighlightReq::File(job) => {
                self.jobs.retain(|old| {
                    old.key != job.key || (old.head == job.head && old.path != job.path)
                });
                self.jobs.push_back(job);
            }
            HighlightReq::Cancel { key, path } => {
                self.jobs.retain(|old| old.key != key || old.path != path);
            }
        }
    }

    /// The oldest waiting job.
    fn pop(&mut self) -> Option<HighlightJob> {
        self.jobs.pop_front()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::domain::diff::{Piece, Syntax};
    use crate::domain::keys::PrKey;
    use crate::workers::testkit;

    fn key(number: u64) -> PrKey {
        PrKey {
            repo: testkit::repo(),
            number,
        }
    }

    fn job(number: u64, head: &str, path: &str) -> HighlightJob {
        HighlightJob {
            key: key(number),
            head: head.into(),
            path: path.into(),
            patch: "@@ -1 +1 @@\n-let a = 1;\n+let b = 2;".into(),
        }
    }

    /// The pull request, head and path of each waiting job.
    fn waiting(queue: &Queue) -> Vec<(u64, &str, &str)> {
        queue
            .jobs
            .iter()
            .map(|j| (j.key.number, j.head.as_str(), j.path.as_str()))
            .collect()
    }

    #[test]
    fn a_new_head_or_a_cancel_drops_the_waiting_jobs() {
        let mut queue = Queue::default();
        queue.push(HighlightReq::File(job(1, "h1", "a.rs")));
        queue.push(HighlightReq::File(job(1, "h1", "b.rs")));
        queue.push(HighlightReq::File(job(2, "h1", "a.rs")));
        queue.push(HighlightReq::File(job(1, "h1", "a.rs")));
        assert_eq!(
            waiting(&queue),
            [(1, "h1", "b.rs"), (2, "h1", "a.rs"), (1, "h1", "a.rs")],
            "a job again goes to the end, once"
        );
        queue.push(HighlightReq::File(job(1, "h2", "c.rs")));
        assert_eq!(
            waiting(&queue),
            [(2, "h1", "a.rs"), (1, "h2", "c.rs")],
            "the new head drops the old head of the same pull request"
        );
        queue.push(HighlightReq::Cancel {
            key: key(2),
            path: "a.rs".into(),
        });
        queue.push(HighlightReq::Cancel {
            key: key(1),
            path: "other.rs".into(),
        });
        assert_eq!(waiting(&queue), [(1, "h2", "c.rs")]);
        assert_eq!(queue.pop().map(|j| j.path), Some("c.rs".into()));
        assert!(queue.is_empty());
    }

    #[tokio::test]
    async fn the_worker_sends_the_tokens_of_each_job() {
        let (ctx, mut harness) = testkit::ctx("http://127.0.0.1:1");
        let worker = tokio::spawn(run(ctx));
        harness
            .highlight
            .send(HighlightReq::File(job(1, "h1", "a.rs")))
            .await
            .unwrap();
        let Msg::Highlighted(done) = harness.recv().await else {
            panic!("not a highlight");
        };
        assert_eq!((done.key, done.head.as_str()), (key(1), "h1"));
        let lines = done.lines.unwrap();
        assert_eq!(lines.len(), 2, "one entry per code line");
        assert_eq!(
            lines[1][0],
            Piece {
                syntax: Some(Syntax::Keyword),
                changed: false,
                text: "let".into(),
            }
        );
        harness
            .highlight
            .send(HighlightReq::File(job(1, "h1", "notes.unknown")))
            .await
            .unwrap();
        let Msg::Highlighted(done) = harness.recv().await else {
            panic!("not a highlight");
        };
        let lines = done.lines.unwrap();
        assert!(
            lines.iter().flatten().all(|p| p.syntax.is_none()),
            "an unknown language has no syntax colors, only changed words"
        );
        drop(harness);
        worker.await.unwrap();
    }
}
