//! The serde schema of the configuration file. Every section has defaults,
//! so an empty file is a valid configuration.

use std::path::PathBuf;

use serde::{Deserialize, Deserializer};

use crate::domain::keys::RepoKey;

/// One `[[repo]]` entry: a repo to track and its local checkout.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct RepoEntry {
    /// The repo in the `owner/name` form.
    #[serde(deserialize_with = "repo_key")]
    pub name: RepoKey,
    /// The local checkout. `~` expands to the home directory.
    #[serde(default)]
    pub path: Option<PathBuf>,
}

/// The `[refresh]` section: the poll intervals in seconds.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
#[allow(
    clippy::struct_field_names,
    reason = "the suffix names the unit of each interval in the file"
)]
pub(crate) struct RefreshConfig {
    /// The delta sync of the pull requests of the user.
    pub mine_secs: u64,
    /// The delta sync of the pull requests of colleagues.
    pub team_secs: u64,
    /// The full sync of both scopes.
    pub full_secs: u64,
    /// The CI poll of the visible pull requests.
    pub ci_secs: u64,
    /// The jj scan while the active tab shows jj data.
    pub jj_secs: u64,
}

impl Default for RefreshConfig {
    fn default() -> Self {
        Self {
            mine_secs: 60,
            team_secs: 90,
            full_secs: 15 * 60,
            ci_secs: 45,
            jj_secs: 30,
        }
    }
}

/// The `[prefetch]` section: the background fetch of pull request refs
/// into the local repo.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(crate) struct PrefetchConfig {
    pub enabled: bool,
}

impl Default for PrefetchConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// The `[codeview]` section: the Rex pane that opens codeview.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(crate) struct CodeviewConfig {
    /// Zooms the pane so that nvim fills the window.
    pub zoom: bool,
    /// The `rex split` direction.
    pub split: String,
    /// The `rex split --ratio` value in percent. `None` uses the Rex
    /// default.
    pub ratio: Option<u8>,
}

impl Default for CodeviewConfig {
    fn default() -> Self {
        Self {
            zoom: true,
            split: "right".to_owned(),
            ratio: None,
        }
    }
}

/// The `[merge]` section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(crate) struct MergeConfig {}

/// The `[theme]` section.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(crate) struct ThemeConfig {
    /// Draws the rough block borders.
    pub grunge: bool,
    /// Fills empty areas with noise textures.
    pub textures: bool,
    /// Turns the transitions on.
    pub animations: bool,
    /// The duration of a transition.
    pub animation_ms: u64,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            grunge: true,
            textures: true,
            animations: true,
            animation_ms: 160,
        }
    }
}

/// The `[terminal]` section: the terminal status sequences. `None` turns a
/// sequence on inside Rex and Ghostty only. `Some` forces it on or off.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(crate) struct TerminalConfig {
    /// OSC 7501, the program status.
    pub program_status: Option<bool>,
    /// OSC 9;4, the progress bar.
    pub progress: Option<bool>,
}

/// The `[workspaces]` section.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(crate) struct WorkspacesConfig {
    /// The directory of the jj workspaces other than `default`:
    /// `<root>/<repo name>/<workspace name>`. `~` expands to the home
    /// directory.
    pub root: PathBuf,
}

impl Default for WorkspacesConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("~/code/workspaces"),
        }
    }
}

/// Reads `owner/name` into a `RepoKey`. `None` when the text has not
/// exactly one `/` or a part is empty.
pub(crate) fn parse_repo_name(text: &str) -> Option<RepoKey> {
    let (owner, name) = text.split_once('/')?;
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return None;
    }
    Some(RepoKey {
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}

/// Deserializes an `owner/name` string into a `RepoKey`.
fn repo_key<'de, D: Deserializer<'de>>(deserializer: D) -> Result<RepoKey, D::Error> {
    let text = String::deserialize(deserializer)?;
    parse_repo_name(&text).ok_or_else(|| {
        serde::de::Error::custom(format!("repo `{text}` is not in the owner/name form"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_repo_name_cases() {
        let key = |owner: &str, name: &str| RepoKey {
            owner: owner.into(),
            name: name.into(),
        };
        assert_eq!(
            parse_repo_name("chaosteil/chaoshub"),
            Some(key("chaosteil", "chaoshub"))
        );
        for bad in ["chaoshub", "/chaoshub", "chaosteil/", "a/b/c", ""] {
            assert_eq!(parse_repo_name(bad), None, "{bad}");
        }
    }
}
