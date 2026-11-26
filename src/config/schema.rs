//! The serde schema of the configuration file. Every section has defaults,
//! so an empty file is a valid configuration.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Deserializer};

use crate::domain::agent::Backend;
use crate::domain::keys::RepoKey;
use crate::domain::merge::{MergeMethod, StackOverride};
use crate::domain::pr::PullRequest;
use crate::domain::resume::TOKEN_ENV;

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

/// The `[filters]` section: which pull requests appear in the lists.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct FiltersConfig {
    /// The maximum time since a pull request was created.
    #[serde(default, deserialize_with = "max_age")]
    pub max_age: Option<TimeDelta>,
}

impl FiltersConfig {
    /// The lower edge of the age window at `now`. None when no limit is
    /// set or the window covers every supported timestamp.
    pub(crate) fn created_since(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.max_age.and_then(|age| now.checked_sub_signed(age))
    }

    /// True when the pull request passes the age limit at `now`.
    /// An old cache without a creation time waits for a sync when a
    /// limit is set.
    pub(crate) fn matches(&self, pr: &PullRequest, now: DateTime<Utc>) -> bool {
        self.created_since(now).is_none_or(|since| {
            pr.head
                .value
                .created_at
                .is_some_and(|created| created >= since)
        })
    }
}

/// Reads a positive duration such as `2 weeks` or `14 days`.
fn max_age<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<TimeDelta>, D::Error> {
    let Some(text) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };
    let duration = humantime::parse_duration(&text).map_err(serde::de::Error::custom)?;
    if duration.is_zero() {
        return Err(serde::de::Error::custom(
            "max_age must be greater than zero",
        ));
    }
    TimeDelta::from_std(duration)
        .map(Some)
        .map_err(|_| serde::de::Error::custom("max_age is too large"))
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

/// The `[codeview]` section: the Rex pane that opens codeview, and the
/// directory of the codeview comment files.
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
    /// The directory of the codeview comment files. `~` expands to the
    /// home directory. `None` uses the default of codeview:
    /// `<nvim data dir>/codeview/review`.
    pub comments_dir: Option<PathBuf>,
}

impl Default for CodeviewConfig {
    fn default() -> Self {
        Self {
            zoom: true,
            split: "right".to_owned(),
            ratio: None,
            comments_dir: None,
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

/// The `[conversation]` section: the conversation pane of the pull
/// request detail.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(crate) struct ConversationConfig {
    /// The order of the entries, and of the comments in a thread.
    pub order: ConversationOrder,
}

/// A `[conversation] order` in the configuration file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConversationOrder {
    /// The time order: the oldest entry at the top.
    #[default]
    OldestFirst,
    /// The reverse of the time order: the newest entry at the top.
    NewestFirst,
}

/// The `[actions.copy_stack_urls]` table: the settings of the action
/// that copies the urls of a stack. The action reads the table, and
/// `Config::parse` also reads it, so a bad value stops the load.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(crate) struct CopyStackUrlsConfig {
    /// The order of the urls, one url on each line.
    pub order: StackUrlsOrder,
}

/// An `[actions.copy_stack_urls] order` in the configuration file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StackUrlsOrder {
    /// The order of the Stack pane: position 1, the bottom of the stack,
    /// first.
    #[default]
    BottomFirst,
    /// The order of the list: the top of the stack first.
    TopFirst,
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
    /// The look: punk, grunge or plain.
    pub style: ThemeStyleSetting,
    /// The colors: Pierre or the 16 ANSI colors.
    pub palette: PaletteSetting,
    /// The primary color in place of the one of the palette: a hex color
    /// or an ANSI color name. `None` keeps the color of the palette.
    #[serde(deserialize_with = "theme_color")]
    pub primary: Option<ColorSetting>,
    /// Draws the rough block borders. Grunge only.
    pub grunge: bool,
    /// Fills empty areas with noise textures. Grunge only.
    pub textures: bool,
    /// Turns the transitions on.
    pub animations: bool,
    /// The duration of a transition.
    pub animation_ms: u64,
    /// Shows the key hint bar above the status bar.
    pub key_hints: bool,
    /// Shows the header row with the column names above the list.
    pub column_headers: bool,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            style: ThemeStyleSetting::Punk,
            palette: PaletteSetting::Pierre,
            primary: None,
            grunge: true,
            textures: true,
            animations: true,
            animation_ms: 120,
            key_hints: true,
            column_headers: true,
        }
    }
}

/// A color of the `[theme]` section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorSetting {
    /// A hex color, as `"#ff5f87"` or `"#f58"`.
    Rgb(u8, u8, u8),
    /// One of the 16 ANSI colors by its number: 0 is `black`, 15 is
    /// `bright_white`.
    Ansi(u8),
}

/// The names of the 16 ANSI colors, in the order of their numbers.
pub(crate) const ANSI_COLOR_NAMES: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "bright_black",
    "bright_red",
    "bright_green",
    "bright_yellow",
    "bright_blue",
    "bright_magenta",
    "bright_cyan",
    "bright_white",
];

impl ColorSetting {
    /// Reads `#rrggbb`, `#rgb`, or the name of an ANSI color, as `blue` or
    /// `bright_magenta`. The error tells which values are valid.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        if let Some(hex) = text.strip_prefix('#') {
            return parse_hex(hex).ok_or_else(|| {
                format!("invalid hex color `{text}`: use `#rrggbb` or `#rgb`, as `#ff5f87`")
            });
        }
        ANSI_COLOR_NAMES
            .iter()
            .position(|name| *name == text)
            .and_then(|i| u8::try_from(i).ok())
            .map(Self::Ansi)
            .ok_or_else(|| {
                format!(
                    "unknown color `{text}`: use a hex color as `#ff5f87`, or one of {}",
                    ANSI_COLOR_NAMES.join(", ")
                )
            })
    }
}

/// The color of the hex digits after `#`: six digits, or three digits
/// that each count twice (`f80` is `ff8800`).
fn parse_hex(hex: &str) -> Option<ColorSetting> {
    if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize, len: usize| u8::from_str_radix(&hex[i..i + len], 16).ok();
    match hex.len() {
        6 => Some(ColorSetting::Rgb(byte(0, 2)?, byte(2, 2)?, byte(4, 2)?)),
        3 => Some(ColorSetting::Rgb(
            byte(0, 1)? * 0x11,
            byte(1, 1)? * 0x11,
            byte(2, 1)? * 0x11,
        )),
        _ => None,
    }
}

/// Reads a color of the `[theme]` section, as `"#ff5f87"` or `"blue"`.
fn theme_color<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ColorSetting>, D::Error> {
    let text = String::deserialize(deserializer)?;
    ColorSetting::parse(&text)
        .map(Some)
        .map_err(serde::de::Error::custom)
}

/// A `[theme] style` in the configuration file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ThemeStyleSetting {
    /// Rough block borders and textures.
    Grunge,
    /// Angular cuts, diagonal accents, inverted headers and a selection
    /// band.
    #[default]
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

/// The `[terminal]` section: the terminal status sequences, the program
/// resume and the mouse. For a status sequence, `None` turns it on inside
/// Rex and Ghostty only, and `Some` forces it on or off.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(crate) struct TerminalConfig {
    /// OSC 2, the terminal title.
    pub title: Option<bool>,
    /// OSC 7501, the program status.
    pub program_status: Option<bool>,
    /// OSC 9;4, the progress bar.
    pub progress: Option<bool>,
    /// True when chaoshub reads the mouse. False leaves the mouse, and
    /// so the text selection, to the terminal.
    pub mouse: bool,
    /// OSC 7577, the program resume. chaoshub reports only when the
    /// terminal sets `OSC7577_TOKEN`.
    #[allow(dead_code, reason = "the resume wiring in run uses it")]
    pub resume: bool,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            title: None,
            program_status: None,
            progress: None,
            mouse: true,
            resume: true,
        }
    }
}

/// The tab ids that `[notify] tabs` accepts.
pub(crate) const NOTIFY_TABS: &[&str] = &["mine", "review", "team"];

/// The `[notify]` section: the terminal notification of new pull
/// requests that are ready for review.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub(crate) struct NotifyConfig {
    /// `None` turns the notification on inside Rex and Ghostty only, the
    /// same as `[terminal] program_status`. `Some` forces it on or off.
    pub enabled: Option<bool>,
    /// The ids of the tabs whose new ready pull requests are announced.
    #[serde(deserialize_with = "notify_tabs")]
    pub tabs: Vec<String>,
    /// How long chaoshub waits after a new pull request before it
    /// announces the new pull requests. Each new pull request starts the
    /// wait again, up to 5 times this value. Zero announces at once.
    #[serde(deserialize_with = "debounce")]
    pub debounce: TimeDelta,
    /// True when the notice also goes out as a desktop notification
    /// (OSC 777), with the text of the notice.
    pub desktop: bool,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            enabled: None,
            tabs: vec!["team".to_owned(), "review".to_owned()],
            debounce: TimeDelta::seconds(15),
            desktop: true,
        }
    }
}

/// Reads tab ids and checks each one against `NOTIFY_TABS`.
fn notify_tabs<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    let tabs = Vec::<String>::deserialize(deserializer)?;
    if let Some(tab) = tabs.iter().find(|tab| !NOTIFY_TABS.contains(&tab.as_str())) {
        return Err(serde::de::Error::custom(format!(
            "unknown tab {tab:?} in notify.tabs, expected one of {}",
            NOTIFY_TABS.join(", ")
        )));
    }
    Ok(tabs)
}

/// Reads a duration such as `15s` or `2 minutes`. Zero is valid.
fn debounce<'de, D: Deserializer<'de>>(deserializer: D) -> Result<TimeDelta, D::Error> {
    let text = String::deserialize(deserializer)?;
    let duration = humantime::parse_duration(&text)
        .map_err(|err| serde::de::Error::custom(format!("invalid notify.debounce: {err}")))?;
    TimeDelta::from_std(duration)
        .map_err(|_| serde::de::Error::custom("notify.debounce is too large"))
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

/// The `[agent]` section: the agent runs. `backend` selects the program
/// that runs the tools. `env` and `timeout_secs` at the top apply to each
/// backend. The `[agent.open_code_review]` and `[agent.pr_agent]` tables
/// hold the settings of each backend. The `env` of a table adds to the
/// top `env`, and its `timeout_secs` replaces the top value.
///
/// The pr-agent keys `command`, `model`, `fallback_models` and
/// `extra_instructions` were at the top of `[agent]` before the
/// `[agent.pr_agent]` table. They are still valid there. A key in both
/// places is an error.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(try_from = "AgentFile")]
pub(crate) struct AgentConfig {
    pub backend: Backend,
    pub open_code_review: OcrConfig,
    pub pr_agent: PrAgentConfig,
}

/// The settings of Open Code Review, with the shared keys of `[agent]`
/// merged in. An empty `provider` or `model` lets `ocr` use its own
/// config (`~/.opencodereview/config.json`) or its env variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OcrConfig {
    /// The program and its first arguments. A string is one program
    /// name.
    pub command: Vec<String>,
    /// The `--provider` of `ocr review`.
    pub provider: String,
    /// The `--model` of `ocr review`.
    pub model: String,
    /// More arguments of `ocr review`, for example `["--effort", "high"]`.
    pub args: Vec<String>,
    /// The env of the ocr process: a variable name to a value.
    pub env: BTreeMap<String, EnvValue>,
    /// The time after which chaoshub stops a run.
    pub timeout_secs: u64,
    /// Extra instructions for the review, after the title and the body of
    /// the pull request in the background of ocr.
    pub extra_instructions: String,
}

impl Default for OcrConfig {
    fn default() -> Self {
        Self {
            command: vec!["ocr".to_owned()],
            provider: String::new(),
            model: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
            timeout_secs: AGENT_TIMEOUT_SECS,
            extra_instructions: String::new(),
        }
    }
}

/// The settings of pr-agent, with the shared keys of `[agent]` merged
/// in. chaoshub does not read the model names. It gives them to
/// pr-agent, and pr-agent gives them to `LiteLLM`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PrAgentConfig {
    /// The program and its first arguments. A string is one program
    /// name.
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

/// The default of `[agent] timeout_secs`.
const AGENT_TIMEOUT_SECS: u64 = 900;

impl Default for PrAgentConfig {
    fn default() -> Self {
        Self {
            command: vec!["pr-agent".to_owned()],
            model: "anthropic/claude-sonnet-5-5".to_owned(),
            fallback_models: Vec::new(),
            env: BTreeMap::new(),
            timeout_secs: AGENT_TIMEOUT_SECS,
            extra_instructions: ExtraInstructions::default(),
        }
    }
}

/// The `[agent]` section as it is in the file. `None` is a key that the
/// file does not set.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AgentFile {
    backend: Backend,
    env: BTreeMap<String, EnvValue>,
    timeout_secs: Option<u64>,
    /// The old place of the pr-agent keys.
    #[serde(flatten)]
    legacy: PrAgentKeys,
    open_code_review: OcrFile,
    pr_agent: PrAgentFile,
}

/// The `[agent.open_code_review]` table as it is in the file.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct OcrFile {
    #[serde(deserialize_with = "some_command_words")]
    command: Option<Vec<String>>,
    provider: String,
    model: String,
    args: Vec<String>,
    env: BTreeMap<String, EnvValue>,
    timeout_secs: Option<u64>,
    extra_instructions: String,
}

/// The `[agent.pr_agent]` table as it is in the file.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PrAgentFile {
    #[serde(flatten)]
    keys: PrAgentKeys,
    env: BTreeMap<String, EnvValue>,
    timeout_secs: Option<u64>,
}

/// The keys of pr-agent that can also be at the top of `[agent]`.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PrAgentKeys {
    #[serde(deserialize_with = "some_command_words")]
    command: Option<Vec<String>>,
    model: Option<String>,
    fallback_models: Option<Vec<String>>,
    extra_instructions: Option<ExtraInstructions>,
}

impl TryFrom<AgentFile> for AgentConfig {
    type Error = String;

    fn try_from(file: AgentFile) -> Result<Self, String> {
        let timeout_secs = file.timeout_secs.unwrap_or(AGENT_TIMEOUT_SECS);
        let keys = merge_pr_agent_keys(file.legacy, file.pr_agent.keys)?;
        let defaults = PrAgentConfig::default();
        let pr_agent = PrAgentConfig {
            command: keys.command.unwrap_or(defaults.command),
            model: keys.model.unwrap_or(defaults.model),
            fallback_models: keys.fallback_models.unwrap_or(defaults.fallback_models),
            env: merge_env(&file.env, file.pr_agent.env),
            timeout_secs: file.pr_agent.timeout_secs.unwrap_or(timeout_secs),
            extra_instructions: keys.extra_instructions.unwrap_or_default(),
        };
        let ocr = file.open_code_review;
        let open_code_review = OcrConfig {
            command: ocr.command.unwrap_or_else(|| OcrConfig::default().command),
            provider: ocr.provider,
            model: ocr.model,
            args: ocr.args,
            env: merge_env(&file.env, ocr.env),
            timeout_secs: ocr.timeout_secs.unwrap_or(timeout_secs),
            extra_instructions: ocr.extra_instructions,
        };
        Ok(Self {
            backend: file.backend,
            open_code_review,
            pr_agent,
        })
    }
}

/// The pr-agent keys of the top of `[agent]` and of `[agent.pr_agent]`
/// together. A key in both is an error.
fn merge_pr_agent_keys(top: PrAgentKeys, table: PrAgentKeys) -> Result<PrAgentKeys, String> {
    fn one<T>(name: &str, top: Option<T>, table: Option<T>) -> Result<Option<T>, String> {
        match (top, table) {
            (Some(_), Some(_)) => Err(format!(
                "agent.{name} and agent.pr_agent.{name} are both set: keep only agent.pr_agent.{name}"
            )),
            (top, table) => Ok(table.or(top)),
        }
    }
    Ok(PrAgentKeys {
        command: one("command", top.command, table.command)?,
        model: one("model", top.model, table.model)?,
        fallback_models: one(
            "fallback_models",
            top.fallback_models,
            table.fallback_models,
        )?,
        extra_instructions: one(
            "extra_instructions",
            top.extra_instructions,
            table.extra_instructions,
        )?,
    })
}

/// The shared env of `[agent]` with the env of a backend table on top. A
/// name in both gets the value of the backend table.
fn merge_env(
    shared: &BTreeMap<String, EnvValue>,
    own: BTreeMap<String, EnvValue>,
) -> BTreeMap<String, EnvValue> {
    let mut env = shared.clone();
    env.extend(own);
    env
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

/// An error that stops a run before the backend starts. The text holds the
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

/// Reads each value of `env`. `var` gives the value of a variable in the
/// env of chaoshub. A command runs in `sh -c`. The first value that fails
/// stops the read.
pub(crate) async fn resolve_env(
    env: &BTreeMap<String, EnvValue>,
    var: impl Fn(&str) -> Option<String>,
) -> Result<BTreeMap<String, String>, EnvError> {
    let mut resolved = BTreeMap::new();
    for (name, value) in env {
        let text = match value {
            EnvValue::Literal(text) => text.clone(),
            EnvValue::FromEnv { from_env } => var(from_env).ok_or_else(|| EnvError::Missing {
                name: name.clone(),
                var: from_env.clone(),
            })?,
            EnvValue::Command { command } => run_env_command(name, command).await?,
        };
        resolved.insert(name.clone(), text);
    }
    Ok(resolved)
}

/// Runs `command` in `sh -c` and returns its trimmed standard output.
async fn run_env_command(name: &str, command: &str) -> Result<String, EnvError> {
    let output = tokio::process::Command::new("sh")
        .env_remove(TOKEN_ENV)
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

/// `command_words` for an optional key.
fn some_command_words<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error> {
    command_words(deserializer).map(Some)
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
    fn terminal_resume_defaults_on() {
        let terminal: TerminalConfig = toml::from_str("").unwrap();
        assert!(terminal.resume);
        let terminal: TerminalConfig = toml::from_str("resume = false").unwrap();
        assert!(!terminal.resume);
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
    fn conversation_order_parses() {
        let conversation: ConversationConfig = toml::from_str("").unwrap();
        assert_eq!(conversation.order, ConversationOrder::OldestFirst);
        let conversation: ConversationConfig = toml::from_str("order = \"oldest_first\"").unwrap();
        assert_eq!(conversation.order, ConversationOrder::OldestFirst);
        let conversation: ConversationConfig = toml::from_str("order = \"newest_first\"").unwrap();
        assert_eq!(conversation.order, ConversationOrder::NewestFirst);
        for bad in ["newest", "NewestFirst", "newest-first", ""] {
            let text = format!("order = {bad:?}");
            assert!(
                toml::from_str::<ConversationConfig>(&text).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn copy_stack_urls_order_parses() {
        let copy: CopyStackUrlsConfig = toml::from_str("").unwrap();
        assert_eq!(copy.order, StackUrlsOrder::BottomFirst);
        let copy: CopyStackUrlsConfig = toml::from_str("order = \"bottom_first\"").unwrap();
        assert_eq!(copy.order, StackUrlsOrder::BottomFirst);
        let copy: CopyStackUrlsConfig = toml::from_str("order = \"top_first\"").unwrap();
        assert_eq!(copy.order, StackUrlsOrder::TopFirst);
        for bad in ["top", "TopFirst", "top-first", ""] {
            let text = format!("order = {bad:?}");
            assert!(
                toml::from_str::<CopyStackUrlsConfig>(&text).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn theme_style_parses() {
        let theme: ThemeConfig = toml::from_str("style = \"punk\"").unwrap();
        assert_eq!(theme.style, ThemeStyleSetting::Punk);
        assert!(theme.textures);
        let theme: ThemeConfig = toml::from_str("").unwrap();
        assert_eq!(theme.style, ThemeStyleSetting::Punk);
        assert_eq!(ThemeConfig::default().style, ThemeStyleSetting::Punk);
        assert!(toml::from_str::<ThemeConfig>("style = \"neon\"").is_err());
    }

    #[test]
    fn hex_colors_parse() {
        let parse = ColorSetting::parse;
        assert_eq!(parse("#ff5f87"), Ok(ColorSetting::Rgb(0xff, 0x5f, 0x87)));
        assert_eq!(parse("#FF5F87"), Ok(ColorSetting::Rgb(0xff, 0x5f, 0x87)));
        assert_eq!(parse("#f80"), Ok(ColorSetting::Rgb(0xff, 0x88, 0x00)));
        assert_eq!(parse("#000"), Ok(ColorSetting::Rgb(0, 0, 0)));
    }

    #[test]
    fn ansi_color_names_parse() {
        let parse = ColorSetting::parse;
        assert_eq!(parse("black"), Ok(ColorSetting::Ansi(0)));
        assert_eq!(parse("blue"), Ok(ColorSetting::Ansi(4)));
        assert_eq!(parse("white"), Ok(ColorSetting::Ansi(7)));
        assert_eq!(parse("bright_black"), Ok(ColorSetting::Ansi(8)));
        assert_eq!(parse("bright_magenta"), Ok(ColorSetting::Ansi(13)));
        assert_eq!(parse("bright_white"), Ok(ColorSetting::Ansi(15)));
    }

    #[test]
    fn other_colors_do_not_parse() {
        for text in [
            "",
            "#",
            "#ff5f8",
            "#ff5f877",
            "#ggg",
            "#+12",
            "#é12",
            "ff5f87",
            "Blue",
            "lightblue",
            "pink",
        ] {
            assert!(ColorSetting::parse(text).is_err(), "{text:?}");
        }
        let err = ColorSetting::parse("pink").unwrap_err();
        assert!(err.contains("unknown color `pink`"), "{err}");
        assert!(err.contains("bright_magenta"), "{err}");
        let err = ColorSetting::parse("#12").unwrap_err();
        assert!(err.contains("invalid hex color `#12`"), "{err}");
    }

    #[test]
    fn theme_primary_parses() {
        let theme: ThemeConfig = toml::from_str("").unwrap();
        assert_eq!(theme.primary, None);
        let theme: ThemeConfig = toml::from_str("primary = \"#ff5f87\"").unwrap();
        assert_eq!(theme.primary, Some(ColorSetting::Rgb(0xff, 0x5f, 0x87)));
        let theme: ThemeConfig = toml::from_str("primary = \"yellow\"").unwrap();
        assert_eq!(theme.primary, Some(ColorSetting::Ansi(3)));
        let err = toml::from_str::<ThemeConfig>("primary = \"neon\"").unwrap_err();
        assert!(err.to_string().contains("unknown color `neon`"), "{err}");
        assert!(toml::from_str::<ThemeConfig>("primary = 3").is_err());
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
        assert_eq!(agent, AgentConfig::default());
        assert_eq!(agent.backend, Backend::OpenCodeReview);
        let ocr = agent.open_code_review;
        assert_eq!(ocr.command, ["ocr"]);
        assert_eq!((ocr.provider.as_str(), ocr.model.as_str()), ("", ""));
        assert!(ocr.args.is_empty() && ocr.env.is_empty());
        assert_eq!(ocr.timeout_secs, 900);
        assert_eq!(ocr.extra_instructions, "");
        let pr_agent = agent.pr_agent;
        assert_eq!(pr_agent.command, ["pr-agent"]);
        assert_eq!(pr_agent.model, "anthropic/claude-sonnet-5-5");
        assert_eq!(pr_agent.fallback_models, Vec::<String>::new());
        assert!(pr_agent.env.is_empty());
        assert_eq!(pr_agent.timeout_secs, 900);
        assert_eq!(pr_agent.extra_instructions, ExtraInstructions::default());
    }

    #[test]
    fn agent_command_is_a_string_or_a_list() {
        let agent: AgentConfig = toml::from_str("command = \"pr-agent\"").unwrap();
        assert_eq!(agent.pr_agent.command, ["pr-agent"]);
        let agent: AgentConfig = toml::from_str(
            "[pr_agent]\ncommand = [\"uvx\", \"--from\", \"pr-agent\", \"pr-agent\"]",
        )
        .unwrap();
        assert_eq!(
            agent.pr_agent.command,
            ["uvx", "--from", "pr-agent", "pr-agent"]
        );
    }

    #[test]
    fn the_old_top_keys_and_the_pr_agent_table_give_the_same_settings() {
        let old = concat!(
            "command = \"pr-agent\"\n",
            "model = \"openai/gpt-6.1-sol\"\n",
            "fallback_models = [\"openai/gpt-6-luna\"]\n",
            "extra_instructions = { review = \"Focus\" }\n",
        );
        let new = concat!(
            "[pr_agent]\n",
            "command = \"pr-agent\"\n",
            "model = \"openai/gpt-6.1-sol\"\n",
            "fallback_models = [\"openai/gpt-6-luna\"]\n",
            "extra_instructions = { review = \"Focus\" }\n",
        );
        let old: AgentConfig = toml::from_str(old).unwrap();
        let new: AgentConfig = toml::from_str(new).unwrap();
        assert_eq!(old, new);
        assert_eq!(new.pr_agent.model, "openai/gpt-6.1-sol");
        assert_eq!(new.pr_agent.fallback_models, ["openai/gpt-6-luna"]);
        assert_eq!(new.pr_agent.extra_instructions.review, "Focus");
    }

    #[test]
    fn the_backend_selects_ocr_or_pr_agent_and_refuses_other_names() {
        let agent: AgentConfig = toml::from_str("backend = \"pr_agent\"").unwrap();
        assert_eq!(agent.backend, Backend::PrAgent);
        let agent: AgentConfig = toml::from_str("backend = \"open_code_review\"").unwrap();
        assert_eq!(agent.backend, Backend::OpenCodeReview);
        let err = toml::from_str::<AgentConfig>("backend = \"codex\"")
            .unwrap_err()
            .to_string();
        assert!(err.contains("unknown variant `codex`"), "{err}");
        // The old pr-agent keys do not select pr-agent.
        let agent: AgentConfig = toml::from_str("model = \"openai/gpt-6.1-sol\"").unwrap();
        assert_eq!(agent.backend, Backend::OpenCodeReview);
    }

    #[test]
    fn the_ocr_table_parses_and_merges_the_shared_keys() {
        let text = concat!(
            "timeout_secs = 600\n",
            "[env]\n",
            "ANTHROPIC_API_KEY = \"shared\"\n",
            "[open_code_review]\n",
            "command = [\"npx\", \"-y\", \"@alibaba-group/open-code-review\"]\n",
            "provider = \"anthropic\"\n",
            "model = \"claude-opus-4-6\"\n",
            "args = [\"--effort\", \"high\"]\n",
            "timeout_secs = 1800\n",
            "extra_instructions = \"Focus on bugs.\"\n",
            "[open_code_review.env]\n",
            "OCR_LLM_TIMEOUT = \"600\"\n",
        );
        let agent: AgentConfig = toml::from_str(text).unwrap();
        let literal = |text: &str| EnvValue::Literal(text.into());
        assert_eq!(
            agent.open_code_review,
            OcrConfig {
                command: vec![
                    "npx".into(),
                    "-y".into(),
                    "@alibaba-group/open-code-review".into()
                ],
                provider: "anthropic".into(),
                model: "claude-opus-4-6".into(),
                args: vec!["--effort".into(), "high".into()],
                env: BTreeMap::from([
                    ("ANTHROPIC_API_KEY".to_owned(), literal("shared")),
                    ("OCR_LLM_TIMEOUT".to_owned(), literal("600")),
                ]),
                timeout_secs: 1800,
                extra_instructions: "Focus on bugs.".into(),
            }
        );
        // The pr-agent table gets the shared keys only.
        assert_eq!(agent.pr_agent.timeout_secs, 600);
        assert_eq!(
            agent.pr_agent.env,
            BTreeMap::from([("ANTHROPIC_API_KEY".to_owned(), literal("shared"))])
        );
    }

    #[test]
    fn a_pr_agent_key_in_both_places_is_an_error() {
        let text = "model = \"a\"\n[pr_agent]\nmodel = \"b\"\n";
        let err = toml::from_str::<AgentConfig>(text).unwrap_err().to_string();
        assert!(
            err.contains("agent.model and agent.pr_agent.model are both set"),
            "{err}"
        );
    }

    #[test]
    fn the_pr_agent_table_adds_env_and_replaces_the_timeout() {
        let text = concat!(
            "timeout_secs = 600\n",
            "[env]\n",
            "A = \"shared\"\n",
            "B = \"shared\"\n",
            "[pr_agent.env]\n",
            "B = \"own\"\n",
            "C = \"own\"\n",
        );
        let agent: AgentConfig = toml::from_str(text).unwrap();
        let literal = |text: &str| EnvValue::Literal(text.into());
        assert_eq!(
            agent.pr_agent.env,
            BTreeMap::from([
                ("A".to_owned(), literal("shared")),
                ("B".to_owned(), literal("own")),
                ("C".to_owned(), literal("own")),
            ])
        );
        assert_eq!(agent.pr_agent.timeout_secs, 600);
        let agent: AgentConfig =
            toml::from_str("timeout_secs = 600\n[pr_agent]\ntimeout_secs = 60\n").unwrap();
        assert_eq!(agent.pr_agent.timeout_secs, 60);
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
            agent.pr_agent.env,
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

    /// The env of `text`, a `[env]` table.
    fn env_of(text: &str) -> BTreeMap<String, EnvValue> {
        toml::from_str::<AgentConfig>(text).unwrap().pr_agent.env
    }

    #[tokio::test]
    async fn agent_env_resolves_the_three_forms() {
        let text = concat!(
            "[env]\n",
            "A = \"literal\"\n",
            "B = { from_env = \"SOURCE\" }\n",
            "C = { command = \"printf '  from command \\\\n'\" }\n",
        );
        let env = resolve_env(&env_of(text), |var| {
            (var == "SOURCE").then(|| "from env".to_owned())
        })
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
        let env = env_of("[env]\nB = { from_env = \"NOPE\" }");
        let err = resolve_env(&env, |_| None).await.unwrap_err();
        assert!(matches!(err, EnvError::Missing { .. }), "{err}");
        assert_eq!(err.to_string(), "env B: the variable NOPE is not set");
    }

    #[tokio::test]
    async fn agent_env_fails_on_a_failed_command() {
        let env = env_of("[env]\nC = { command = \"echo secret; echo no key >&2; exit 3\" }");
        let err = resolve_env(&env, |_| None).await.unwrap_err();
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
