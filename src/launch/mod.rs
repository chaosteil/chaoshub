//! Adapter for detached panes: the `Launcher` trait.
#![allow(
    dead_code,
    reason = "the action runtime uses the launcher in a later phase"
)]

pub(crate) mod error;
pub(crate) mod rex;

use futures::future::BoxFuture;

use self::error::LaunchError;
use crate::bus::LaunchRequest;

/// A pane that a launcher started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LaunchHandle {
    /// The label of the pane, unique per launch.
    pub label: String,
    /// The id of the pane. It is the label when the launcher did not
    /// report an id.
    pub block_id: String,
}

/// Starts commands in detached panes beside the TUI, and waits for,
/// focuses and closes them. The methods return boxed futures, so the
/// trait stays usable as `dyn Launcher`.
pub(crate) trait Launcher: Send + Sync {
    /// Starts `req` in a new pane.
    fn launch(&self, req: &LaunchRequest) -> BoxFuture<'_, Result<LaunchHandle, LaunchError>>;
    /// Waits until the command of the pane exits, and returns its exit
    /// code.
    fn wait(&self, handle: &LaunchHandle) -> BoxFuture<'_, Result<i32, LaunchError>>;
    /// True while the pane exists.
    fn alive(&self, handle: &LaunchHandle) -> BoxFuture<'_, bool>;
    /// Moves the focus to the pane.
    fn focus(&self, handle: &LaunchHandle) -> BoxFuture<'_, Result<(), LaunchError>>;
    /// Closes the pane.
    fn close(&self, handle: &LaunchHandle) -> BoxFuture<'_, Result<(), LaunchError>>;
}
