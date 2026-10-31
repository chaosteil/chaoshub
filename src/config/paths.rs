//! The config, cache and state directories, from `etcetera`, and `~`
//! expansion.
//!
//! The base strategy gives XDG paths on macOS too, so the files live in
//! `~/.config`, `~/.cache` and `~/.local/state`, not in `~/Library`.

use std::path::{Path, PathBuf};

use etcetera::BaseStrategy;

/// The state directory of chaoshub: `$XDG_STATE_HOME/chaoshub`, by default
/// `~/.local/state/chaoshub`. The base strategy gives XDG paths on macOS
/// too. `None` when the home directory is unknown.
pub fn state_dir() -> Option<PathBuf> {
    let strategy = etcetera::choose_base_strategy().ok()?;
    Some(strategy.state_dir()?.join("chaoshub"))
}

/// The config directory of chaoshub: `$XDG_CONFIG_HOME/chaoshub`, by
/// default `~/.config/chaoshub`. `None` when the home directory is unknown.
pub(crate) fn config_dir() -> Option<PathBuf> {
    let strategy = etcetera::choose_base_strategy().ok()?;
    Some(strategy.config_dir().join("chaoshub"))
}

/// The cache directory of chaoshub: `$XDG_CACHE_HOME/chaoshub`, by default
/// `~/.cache/chaoshub`. `None` when the home directory is unknown.
pub(crate) fn cache_dir() -> Option<PathBuf> {
    let strategy = etcetera::choose_base_strategy().ok()?;
    Some(strategy.cache_dir().join("chaoshub"))
}

/// The configuration file: `config.toml` in the config directory.
pub(crate) fn config_file() -> Option<PathBuf> {
    Some(config_dir()?.join("config.toml"))
}

/// The SQLite cache: `chaoshub.db` in the cache directory.
pub(crate) fn cache_db() -> Option<PathBuf> {
    Some(cache_dir()?.join("chaoshub.db"))
}

/// The home directory of the user. `None` when it is unknown.
pub(crate) fn home_dir() -> Option<PathBuf> {
    let strategy = etcetera::choose_base_strategy().ok()?;
    Some(strategy.home_dir().to_path_buf())
}

/// Replaces a leading `~` component of `path` with `home`. Other paths,
/// and `~user` forms, stay as they are.
pub(crate) fn expand_tilde(path: &Path, home: &Path) -> PathBuf {
    match path.strip_prefix("~") {
        Ok(rest) => home.join(rest),
        Err(_) => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_tilde_cases() {
        let home = Path::new("/home/me");
        let cases = [
            ("~", "/home/me"),
            ("~/code/chaoshub", "/home/me/code/chaoshub"),
            ("/abs/path", "/abs/path"),
            ("relative/path", "relative/path"),
            ("~other/code", "~other/code"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                expand_tilde(Path::new(input), home),
                PathBuf::from(expected),
                "{input}"
            );
        }
    }

    #[test]
    fn files_sit_in_the_chaoshub_directories() {
        let config = config_file().unwrap();
        assert!(config.ends_with("chaoshub/config.toml"));
        let db = cache_db().unwrap();
        assert!(db.ends_with("chaoshub/chaoshub.db"));
        assert!(state_dir().unwrap().ends_with("chaoshub"));
    }
}
