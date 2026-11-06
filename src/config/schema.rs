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

/// The `[detail]` section: where `enter` opens the detail of a pull
/// request.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(crate) struct DetailConfig {
    /// A Rex pane or the sidebar.
    pub mode: DetailMode,
    /// The `rex split` direction of the pane.
    pub split: String,
    /// The `rex split --ratio` value of the pane in percent. `None` uses
    /// the Rex default.
    pub ratio: Option<u8>,
}

impl Default for DetailConfig {
    fn default() -> Self {
        Self {
            mode: DetailMode::Auto,
            split: "right".to_owned(),
            ratio: None,
        }
    }
}

/// A `[detail] mode` in the configuration file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DetailMode {
    /// Opens a pane inside Rex, and the sidebar elsewhere.
    #[default]
    Auto,
    /// Always opens the pane. Without Rex, the pane runs in the
    /// foreground.
    Pane,
    /// Always opens the sidebar.
    Sidebar,
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

/// The `[terminal]` section: the terminal status sequences and the
/// mouse. For a sequence, `None` turns it on inside Rex and Ghostty only,
/// and `Some` forces it on or off.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(crate) struct TerminalConfig {
    /// OSC 7501, the program status.
    pub program_status: Option<bool>,
    /// OSC 9;4, the progress bar.
    pub progress: Option<bool>,
    /// True when chaoshub reads the mouse. False leaves the mouse, and
    /// so the text selection, to the terminal.
    pub mouse: bool,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            program_status: None,
            progress: None,
            mouse: true,
        }
    }
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

/// The `[agent]` section: the pr-agent runs. chaoshub does not read the
/// model names. It gives them to pr-agent, and pr-agent gives them to
/// `LiteLLM`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(crate) struct AgentConfig {
    /// The program and its first arguments. A string is one program
    /// name.
    #[serde(deserialize_with = "command_words")]
    pub command: Vec<String>,
    /// The `LiteLLM` name of the model.
    pub model: String,
    /// The `LiteLLM` names of the models to try when the model fails.
    pub fallback_models: Vec<String>,
    /// The env of the pr-agent process: a variable name to a value.
    pub env: BTreeMap<String, EnvValue>,
    /// The time after which chaoshub stops a run.
    pub timeout_secs: u64,
    /// The extra instructions that each tool gives to the model.
    pub extra_instructions: ExtraInstructions,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            command: vec!["pr-agent".to_owned()],
            model: "anthropic/claude-sonnet-5-5".to_owned(),
            fallback_models: Vec::new(),
            env: BTreeMap::new(),
            timeout_secs: 900,
            extra_instructions: ExtraInstructions::default(),
        }
    }
}

/// One value of `[agent] env`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub(crate) enum EnvValue {
    /// The value as it is in the file.
    Literal(String),
    /// `{ from_env = "NAME" }`: the value of a variable in the env of
    /// chaoshub.
    FromEnv { from_env: String },
    /// `{ command = "..." }`: the trimmed standard output of a shell
    /// command.
    Command { command: String },
}

/// The `[agent] extra_instructions` table: one text per tool. An empty
/// text gives no extra instructions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(crate) struct ExtraInstructions {
    pub review: String,
    pub improve: String,
    pub describe: String,
    pub ask: String,
}

/// An error that stops a run before pr-agent starts. The text holds the
/// names of the variables, but never their values.
#[derive(Debug, thiserror::Error)]
pub(crate) enum EnvError {
    #[error("env {name}: the variable {var} is not set")]
    Missing { name: String, var: String },
    #[error("env {name}: the command did not start: {source}")]
    Spawn {
        name: String,
        #[source]
        source: std::io::Error,
    },
    #[error("env {name}: the command failed: {stderr}")]
    Failed { name: String, stderr: String },
}

impl AgentConfig {
    /// Reads each value of `env`. `var` gives the value of a variable in
    /// the env of chaoshub. A command runs in `sh -c`. The first value
    /// that fails stops the read.
    pub(crate) async fn resolve_env(
        &self,
        var: impl Fn(&str) -> Option<String>,
    ) -> Result<BTreeMap<String, String>, EnvError> {
        let mut env = BTreeMap::new();
        for (name, value) in &self.env {
            let resolved = match value {
                EnvValue::Literal(text) => text.clone(),
                EnvValue::FromEnv { from_env } => {
                    var(from_env).ok_or_else(|| EnvError::Missing {
                        name: name.clone(),
                        var: from_env.clone(),
                    })?
                }
                EnvValue::Command { command } => run_env_command(name, command).await?,
            };
            env.insert(name.clone(), resolved);
        }
        Ok(env)
    }
}

/// Runs `command` in `sh -c` and returns its trimmed standard output.
async fn run_env_command(name: &str, command: &str) -> Result<String, EnvError> {
    let output = tokio::process::Command::new("sh")
        .args(["-c", command])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|source| EnvError::Spawn {
            name: name.to_owned(),
            source,
        })?;
    if !output.status.success() {
        return Err(EnvError::Failed {
            name: name.to_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// Deserializes a string or a list of strings into a list. A string
/// gives a list of one.
fn command_words<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Words {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Words::deserialize(deserializer)? {
        Words::One(word) => vec![word],
        Words::Many(words) => words,
    })
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
    fn terminal_mouse_defaults_to_on() {
        let terminal: TerminalConfig = toml::from_str("").unwrap();
        assert!(terminal.mouse);
        let terminal: TerminalConfig = toml::from_str("mouse = false").unwrap();
        assert!(!terminal.mouse);
    }

    #[test]
    fn detail_defaults() {
        let detail: DetailConfig = toml::from_str("").unwrap();
        assert_eq!(
            detail,
            DetailConfig {
                mode: DetailMode::Auto,
                split: "right".into(),
                ratio: None,
            }
        );
    }

    #[test]
    fn detail_section_parses() {
        let detail: DetailConfig =
            toml::from_str("mode = \"sidebar\"\nsplit = \"below\"\nratio = 40").unwrap();
        assert_eq!(
            detail,
            DetailConfig {
                mode: DetailMode::Sidebar,
                split: "below".into(),
                ratio: Some(40),
            }
        );
        let detail: DetailConfig = toml::from_str("mode = \"pane\"").unwrap();
        assert_eq!(detail.mode, DetailMode::Pane);
    }

    #[test]
    fn an_unknown_detail_mode_fails() {
        assert!(toml::from_str::<DetailConfig>("mode = \"window\"").is_err());
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
    fn agent_defaults() {
        let agent: AgentConfig = toml::from_str("").unwrap();
        assert_eq!(agent.command, ["pr-agent"]);
        assert_eq!(agent.model, "anthropic/claude-sonnet-5-5");
        assert_eq!(agent.fallback_models, Vec::<String>::new());
        assert!(agent.env.is_empty());
        assert_eq!(agent.timeout_secs, 900);
        assert_eq!(agent.extra_instructions, ExtraInstructions::default());
    }

    #[test]
    fn agent_command_is_a_string_or_a_list() {
        let agent: AgentConfig = toml::from_str("command = \"pr-agent\"").unwrap();
        assert_eq!(agent.command, ["pr-agent"]);
        let agent: AgentConfig =
            toml::from_str("command = [\"uvx\", \"--from\", \"pr-agent\", \"pr-agent\"]").unwrap();
        assert_eq!(agent.command, ["uvx", "--from", "pr-agent", "pr-agent"]);
    }

    #[test]
    fn agent_env_value_forms_parse() {
        let text = concat!(
            "[env]\n",
            "A = \"literal\"\n",
            "B = { from_env = \"HOME\" }\n",
            "C = { command = \"echo hi\" }\n",
        );
        let agent: AgentConfig = toml::from_str(text).unwrap();
        assert_eq!(
            agent.env,
            BTreeMap::from([
                ("A".to_owned(), EnvValue::Literal("literal".into())),
                (
                    "B".to_owned(),
                    EnvValue::FromEnv {
                        from_env: "HOME".into()
                    }
                ),
                (
                    "C".to_owned(),
                    EnvValue::Command {
                        command: "echo hi".into()
                    }
                ),
            ])
        );
        assert!(toml::from_str::<AgentConfig>("[env]\nA = { file = \"x\" }").is_err());
    }

    #[tokio::test]
    async fn agent_env_resolves_the_three_forms() {
        let text = concat!(
            "[env]\n",
            "A = \"literal\"\n",
            "B = { from_env = \"SOURCE\" }\n",
            "C = { command = \"printf '  from command \\\\n'\" }\n",
        );
        let agent: AgentConfig = toml::from_str(text).unwrap();
        let env = agent
            .resolve_env(|var| (var == "SOURCE").then(|| "from env".to_owned()))
            .await
            .unwrap();
        assert_eq!(
            env,
            BTreeMap::from([
                ("A".to_owned(), "literal".to_owned()),
                ("B".to_owned(), "from env".to_owned()),
                ("C".to_owned(), "from command".to_owned()),
            ])
        );
    }

    #[tokio::test]
    async fn agent_env_fails_on_a_missing_variable() {
        let agent: AgentConfig = toml::from_str("[env]\nB = { from_env = \"NOPE\" }").unwrap();
        let err = agent.resolve_env(|_| None).await.unwrap_err();
        assert!(matches!(err, EnvError::Missing { .. }), "{err}");
        assert_eq!(err.to_string(), "env B: the variable NOPE is not set");
    }

    #[tokio::test]
    async fn agent_env_fails_on_a_failed_command() {
        let text = "[env]\nC = { command = \"echo secret; echo no key >&2; exit 3\" }";
        let agent: AgentConfig = toml::from_str(text).unwrap();
        let err = agent.resolve_env(|_| None).await.unwrap_err();
        assert!(matches!(err, EnvError::Failed { .. }), "{err}");
        assert_eq!(err.to_string(), "env C: the command failed: no key");
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
