//! Adapter for jj workspaces and stacks, and git ref prefetch.
#![allow(dead_code, reason = "the workers use the locks in a later phase")]

mod error;
mod git;
mod jj;
pub(crate) mod lock;
mod parse;
