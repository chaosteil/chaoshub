//! Background tasks. They call the adapters and send `bus::Msg`.
#![allow(dead_code, reason = "run spawns the workers in a later phase")]

mod ci;
pub(crate) mod ctx;
mod detail;
mod jj;
mod prefetch;
mod startup;
mod supervisor;
mod sync;
#[cfg(test)]
mod testkit;

use tokio::task::JoinSet;

use crate::workers::ctx::WorkerCtx;
use crate::workers::supervisor::{FIRST_BACKOFF, Worker, supervise};

/// The workers that `spawn_all` starts.
const WORKERS: &[Worker] = &[];

/// Spawns every worker into one `JoinSet`, each one inside its own
/// supervisor. A drop of the set aborts the supervisors and the workers.
pub(crate) fn spawn_all(ctx: WorkerCtx) -> JoinSet<()> {
    let mut set = JoinSet::new();
    let contexts = std::iter::repeat_n(ctx, WORKERS.len());
    for (&worker, ctx) in WORKERS.iter().zip(contexts) {
        set.spawn(supervise(worker, ctx, FIRST_BACKOFF));
    }
    set
}
