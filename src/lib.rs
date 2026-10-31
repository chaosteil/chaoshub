//! chaoshub: a PR tracking TUI for jj, Rex and codeview.
//!
//! All logic lives in the modules of this library. The binary in `main.rs`
//! is thin.
//!
//! The dependency rule. Module dependencies point inward, toward `domain`:
//!
//! ```text
//! domain <- bus <- github | vcs | store | launch <- workers | actions <- app <- ui <- run
//! config: read by app, workers, actions, run.   theme: read by ui and run.   cli: read by main only.
//! osc: reads bus only; read by run and clipboard.   clipboard: read by app (exec).
//! ```
#![forbid(unsafe_code)]

mod actions;
mod app;
mod bus;
pub mod cli;
mod clipboard;
mod config;
mod domain;
mod github;
mod launch;
mod osc;
mod run;
mod store;
mod theme;
mod ui;
mod vcs;
mod workers;
