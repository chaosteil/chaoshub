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

use tokio::task::JoinSet;

use crate::workers::ctx::WorkerCtx;

/// Spawns every worker into one `JoinSet`. It spawns nothing yet.
pub(crate) fn spawn_all(ctx: WorkerCtx) -> JoinSet<()> {
    drop(ctx);
    JoinSet::new()
}
