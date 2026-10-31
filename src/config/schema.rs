//! The serde schema of the configuration file. Every section has defaults,
//! so an empty file is a valid configuration.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Deserializer};

use crate::domain::keys::RepoKey;
use crate::domain::merge::{MergeMethod, StackOverride};

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

/// The `[merge]` section. `Config::parse` also copies its keys into
/// `[actions.merge]`, the table that the merge action reads, unless
/// `[actions.merge]` sets the key. So `[merge.repos."owner/name"]` and
/// `[actions.merge.repos."owner/name"]` both work, and the second wins.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(crate) struct MergeConfig {
    /// The `[merge.repos."owner/name"]` tables: per-repo overrides.
    #[serde(deserialize_with = "repo_map")]
    pub repos: BTreeMap<RepoKey, RepoMerge>,
}

/// The merge overrides of one repo.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(crate) struct RepoMerge {
    /// The merge method. `None` picks squash, then merge, then rebase,
    /// from what the repo permits.
    pub method: Option<MergeMethodSetting>,
    /// Asks GitHub to bypass the branch rules on a merge. Off by default.
    pub bypass_rules: bool,
    /// How the merge action merges more than one entry of a stack.
    pub stack: StackSetting,
}

/// A merge method in the configuration file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MergeMethodSetting {
    Merge,
    Squash,
    Rebase,
}

impl From<MergeMethodSetting> for MergeMethod {
    /// The domain merge method of a `[merge.repos."owner/name"] method`.
    fn from(setting: MergeMethodSetting) -> Self {
        match setting {
            MergeMethodSetting::Merge => MergeMethod::Merge,
            MergeMethodSetting::Squash => MergeMethod::Squash,
            MergeMethodSetting::Rebase => MergeMethod::Rebase,
        }
    }
}

/// The `stack` key of `[merge]`: how a merge of a stack sends its
/// requests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum StackSetting {
    /// Atomic when the squash defaults of the repo are the pull request
    /// title and body, else sequential.
    #[default]
    Auto,
    /// One request for the whole stack.
    Atomic,
    /// One request per entry, bottom first.
    Sequential,
}

impl StackSetting {
    /// The override of the stack strategy. `None` for `auto`.
    pub(crate) fn stack_override(self) -> Option<StackOverride> {
        match self {
            StackSetting::Auto => None,
            StackSetting::Atomic => Some(StackOverride::Atomic),
            StackSetting::Sequential => Some(StackOverride::Sequential),
        }
    }
}

/// The `[theme]` section.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(crate) struct ThemeConfig {
    /// The look: grunge, punk or plain.
    pub style: ThemeStyleSetting,
    /// The colors: Pierre or the 16 ANSI colors.
    pub palette: PaletteSetting,
    /// Draws the rough block borders.
    pub grunge: bool,
    /// Fills empty areas with noise textures. Grunge only.
    pub textures: bool,
    /// Turns the transitions on.
    pub animations: bool,
    /// The duration of a transition.
    pub animation_ms: u64,
    /// Shows the key hint bar above the status bar.
    pub key_hints: bool,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            style: ThemeStyleSetting::Grunge,
            palette: PaletteSetting::Pierre,
            grunge: true,
            textures: true,
            animations: true,
            animation_ms: 160,
            key_hints: true,
        }
    }
}

/// A `[theme] style` in the configuration file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ThemeStyleSetting {
    /// Rough block borders and textures.
    #[default]
    Grunge,
    /// Angular cuts, diagonal accents, inverted headers and a selection
    /// band.
    Punk,
    /// Rounded borders, no textures.
    Plain,
}

/// A `[theme] palette` in the configuration file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PaletteSetting {
    /// The Pierre colors. A 16-color terminal gets the ANSI colors.
    #[default]
    Pierre,
    /// The 16 ANSI colors.
    Ansi16,
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
    /// `<root>/<folder>/<workspace name>`, where `<folder>` is the folder
    /// name of the repo path, for example `chaoshub` for
    /// `~/code/chaoshub`. `~` expands to the home directory.
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

/// Deserializes a table with `owner/name` keys.
fn repo_map<'de, D, V>(deserializer: D) -> Result<BTreeMap<RepoKey, V>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    BTreeMap::<String, V>::deserialize(deserializer)?
        .into_iter()
        .map(|(text, value)| {
            parse_repo_name(&text)
                .map(|key| (key, value))
                .ok_or_else(|| {
                    serde::de::Error::custom(format!("repo `{text}` is not in the owner/name form"))
                })
        })
        .collect()
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

    #[test]
    fn theme_style_parses() {
        let theme: ThemeConfig = toml::from_str("style = \"punk\"").unwrap();
        assert_eq!(theme.style, ThemeStyleSetting::Punk);
        assert!(theme.textures);
        let theme: ThemeConfig = toml::from_str("").unwrap();
        assert_eq!(theme.style, ThemeStyleSetting::Grunge);
        assert!(toml::from_str::<ThemeConfig>("style = \"neon\"").is_err());
    }

    #[test]
    fn theme_palette_parses() {
        let theme: ThemeConfig = toml::from_str("palette = \"ansi16\"").unwrap();
        assert_eq!(theme.palette, PaletteSetting::Ansi16);
        let theme: ThemeConfig = toml::from_str("").unwrap();
        assert_eq!(theme.palette, PaletteSetting::Pierre);
        assert!(toml::from_str::<ThemeConfig>("palette = \"neon\"").is_err());
    }

    #[test]
    fn every_merge_method_setting_converts() {
        let cases = [
            (MergeMethodSetting::Merge, MergeMethod::Merge),
            (MergeMethodSetting::Squash, MergeMethod::Squash),
            (MergeMethodSetting::Rebase, MergeMethod::Rebase),
        ];
        for (setting, method) in cases {
            assert_eq!(MergeMethod::from(setting), method);
        }
    }

    #[test]
    fn every_stack_setting_converts() {
        let cases = [
            (StackSetting::Auto, None),
            (StackSetting::Atomic, Some(StackOverride::Atomic)),
            (StackSetting::Sequential, Some(StackOverride::Sequential)),
        ];
        for (setting, stack_override) in cases {
            assert_eq!(setting.stack_override(), stack_override);
        }
    }
}
