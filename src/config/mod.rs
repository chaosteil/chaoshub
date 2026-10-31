//! Configuration: the file, the paths and the repo of the current
//! directory.
#![allow(
    dead_code,
    reason = "the workers use the configuration in a later phase"
)]

mod discover;
pub(crate) mod paths;
mod schema;

/// The configuration. A placeholder: phase B replaces it with the real
/// configuration.
#[derive(Debug)]
pub(crate) struct Config;
