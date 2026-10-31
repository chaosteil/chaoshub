//! The runtime that executes commands.
//!
//! For now it executes only `Cmd::Quit`. It logs every other command.

use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;

use crate::bus::Cmd;

/// Executes the commands of `update`.
#[derive(Debug)]
pub(crate) struct Runtime {
    /// The tokio runtime of the workers and the action tasks.
    #[allow(dead_code, reason = "phase D spawns the action tasks on it")]
    handle: Handle,
    /// Cancels every task on quit.
    root: CancellationToken,
    /// True after `Cmd::Quit`.
    quit: bool,
}

impl Runtime {
    /// Creates the runtime on the tokio runtime of `handle`.
    pub(crate) fn new(handle: Handle) -> Self {
        Self {
            handle,
            root: CancellationToken::new(),
            quit: false,
        }
    }

    /// Executes one command.
    pub(crate) fn exec(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Quit => {
                self.root.cancel();
                self.quit = true;
            }
            other => tracing::debug!(?other, "command not executed yet"),
        }
    }

    /// True after `Cmd::Quit`. The event loop then stops.
    pub(crate) fn quitting(&self) -> bool {
        self.quit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Scope;

    #[test]
    fn quit_cancels_the_root_token() {
        let tokio = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut runtime = Runtime::new(tokio.handle().clone());
        runtime.exec(Cmd::SyncNow(Scope::Mine));
        assert!(!runtime.quitting());
        runtime.exec(Cmd::Quit);
        assert!(runtime.quitting());
        assert!(runtime.root.is_cancelled());
    }
}
