//! Adapter for the SQLite cache. The connection lives on its own thread.
#![allow(dead_code, reason = "the workers use the store in a later phase")]

mod error;
pub(crate) mod handle;
mod rows;
mod thread;
