//! The config, cache and state directories, from `etcetera`.

use std::path::PathBuf;

use etcetera::BaseStrategy;

/// The state directory of chaoshub: `$XDG_STATE_HOME/chaoshub`, by default
/// `~/.local/state/chaoshub`. The base strategy gives XDG paths on macOS
/// too. `None` when the home directory is unknown.
pub fn state_dir() -> Option<PathBuf> {
    let strategy = etcetera::choose_base_strategy().ok()?;
    Some(strategy.state_dir()?.join("chaoshub"))
}
