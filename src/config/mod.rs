//! Configuration: the file, the paths and the repo of the current
//! directory.
//!
//! The file is `~/.config/chaoshub/config.toml`. A missing file gives the
//! defaults. `load` expands `~` in every path.
#![allow(
    dead_code,
    reason = "the workers use the configuration in a later phase"
)]

pub(crate) mod discover;
pub(crate) mod paths;
pub(crate) mod schema;

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::schema::{
    AgentConfig, CodeviewConfig, DetailConfig, MergeConfig, PrefetchConfig, RefreshConfig,
    RepoEntry, RepoMerge, TerminalConfig, ThemeConfig, WorkspacesConfig,
};
use crate::domain::keys::RepoKey;
use crate::domain::repo::Repo;

/// The configuration of chaoshub. Every field has a default.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub(crate) struct Config {
    /// Overrides the login that the `viewer` query reads.
    pub login: Option<String>,
    /// The repos to track, from the `[[repo]]` entries.
    #[serde(rename = "repo")]
    pub repos: Vec<RepoEntry>,
    pub refresh: RefreshConfig,
    pub prefetch: PrefetchConfig,
    pub codeview: CodeviewConfig,
    /// Where `enter` opens the detail of a pull request.
    pub detail: DetailConfig,
    pub merge: MergeConfig,
    /// The pr-agent runs.
    pub agent: AgentConfig,
    pub theme: ThemeConfig,
    pub terminal: TerminalConfig,
    /// Key binding overrides: an action id or app key name to a key.
    pub keys: BTreeMap<String, String>,
    /// The `[actions.<id>]` tables. Each action reads its own table.
    /// `parse` copies the `method`, `bypass_rules`, `stack` and `repos`
    /// keys of `[merge]` into the `merge` table, so the merge action reads
    /// `[merge.repos."owner/name"]`. A key that `[actions.merge]` sets
    /// stays: `[actions.merge.repos]` overrides `[merge.repos]`.
    pub actions: BTreeMap<String, toml::Table>,
    pub workspaces: WorkspacesConfig,
    /// Teams whose review requests count as requests for the user, in the
    /// `org/team` form.
    pub teams: Vec<String>,
    /// Forces a launcher, for example `rex`. `None` detects it.
    pub launcher: Option<String>,
}

/// The keys of `[merge]` that `parse` copies into `[actions.merge]`.
const MERGE_KEYS: &[&str] = &["method", "bypass_rules", "stack", "repos"];

/// An error that stops the configuration from loading.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ConfigError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("invalid configuration in {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

impl Config {
    /// Parses the text of a configuration file and expands `~` to `home`.
    /// Without a home directory, paths stay as they are. The keys of
    /// `[merge]` go into `[actions.merge]` when that table does not set
    /// them.
    pub(crate) fn parse(text: &str, home: Option<&Path>) -> Result<Self, toml::de::Error> {
        let mut config: Self = toml::from_str(text)?;
        let raw: toml::Table = toml::from_str(text)?;
        config.mirror_merge(&raw);
        if let Some(home) = home {
            config.expand_paths(home);
        }
        Ok(config)
    }

    /// The tracked repos as domain values. The merge settings are unknown
    /// until a sync reads them.
    pub(crate) fn repos(&self) -> Vec<Repo> {
        self.repos
            .iter()
            .map(|entry| Repo {
                key: entry.name.clone(),
                path: entry.path.clone(),
                settings: None,
            })
            .collect()
    }

    /// The merge overrides of a repo. The defaults when the file has
    /// none.
    pub(crate) fn merge_for(&self, repo: &RepoKey) -> RepoMerge {
        self.merge.repos.get(repo).cloned().unwrap_or_default()
    }

    /// The `[actions.<id>]` table of one action.
    pub(crate) fn action(&self, id: &str) -> Option<&toml::Table> {
        self.actions.get(id)
    }

    /// Copies each key of `MERGE_KEYS` from the `[merge]` table of `raw`
    /// into `actions["merge"]`, unless `[actions.merge]` sets the key.
    fn mirror_merge(&mut self, raw: &toml::Table) {
        let Some(toml::Value::Table(merge)) = raw.get("merge") else {
            return;
        };
        for &key in MERGE_KEYS {
            let Some(value) = merge.get(key) else {
                continue;
            };
            let table = self.actions.entry("merge".to_owned()).or_default();
            table.entry(key).or_insert_with(|| value.clone());
        }
    }

    fn expand_paths(&mut self, home: &Path) {
        for entry in &mut self.repos {
            if let Some(path) = &entry.path {
                entry.path = Some(paths::expand_tilde(path, home));
            }
        }
        self.workspaces.root = paths::expand_tilde(&self.workspaces.root, home);
        if let Some(dir) = &self.codeview.comments_dir {
            self.codeview.comments_dir = Some(paths::expand_tilde(dir, home));
        }
    }
}

/// Loads the configuration file at `path`. A missing file gives the
/// defaults.
pub(crate) fn load(path: &Path) -> Result<Config, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let home = paths::home_dir();
    Config::parse(&text, home.as_deref()).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::config::schema::{EnvValue, MergeMethodSetting, StackSetting};

    const SAMPLE: &str = include_str!("testdata/sample.toml");

    fn key(owner: &str, name: &str) -> RepoKey {
        RepoKey {
            owner: owner.into(),
            name: name.into(),
        }
    }

    #[test]
    fn empty_file_gives_the_defaults() {
        let config = Config::parse("", Some(Path::new("/home/me"))).unwrap();
        assert_eq!(config.login, None);
        assert_eq!(config.repos, []);
        assert_eq!(config.refresh, RefreshConfig::default());
        assert_eq!(config.refresh.mine_secs, 60);
        assert_eq!(config.refresh.team_secs, 90);
        assert_eq!(config.refresh.full_secs, 900);
        assert_eq!(config.refresh.ci_secs, 45);
        assert_eq!(config.refresh.jj_secs, 30);
        assert!(config.prefetch.enabled);
        assert!(config.codeview.zoom);
        assert_eq!(config.codeview.split, "right");
        assert_eq!(config.codeview.ratio, None);
        assert_eq!(config.agent, AgentConfig::default());
        assert!(config.theme.grunge);
        assert!(config.theme.textures);
        assert!(config.theme.animations);
        assert_eq!(config.theme.animation_ms, 120);
        assert!(config.theme.column_headers);
        assert_eq!(config.terminal, TerminalConfig::default());
        assert!(config.keys.is_empty());
        assert!(config.actions.is_empty());
        assert_eq!(
            config.workspaces.root,
            PathBuf::from("/home/me/code/workspaces")
        );
        assert_eq!(config.teams, Vec::<String>::new());
        assert_eq!(config.launcher, None);
    }

    #[test]
    fn sample_file_parses() {
        let config = Config::parse(SAMPLE, Some(Path::new("/home/me"))).unwrap();
        assert_eq!(config.login.as_deref(), Some("chaosteil"));
        assert_eq!(config.teams, ["pierredotco/platform"]);
        assert_eq!(config.launcher.as_deref(), Some("rex"));
        assert_eq!(
            config.repos(),
            [
                Repo {
                    key: key("chaosteil", "chaoshub"),
                    path: Some(PathBuf::from("/home/me/code/chaoshub")),
                    settings: None,
                },
                Repo {
                    key: key("pierredotco", "deploy"),
                    path: None,
                    settings: None,
                },
            ]
        );
        assert_eq!(
            config.refresh,
            RefreshConfig {
                mine_secs: 30,
                full_secs: 600,
                ..RefreshConfig::default()
            }
        );
        assert!(!config.prefetch.enabled);
        assert!(!config.codeview.zoom);
        assert_eq!(config.codeview.ratio, Some(70));
        assert!(config.theme.grunge);
        assert!(!config.theme.textures);
        assert_eq!(config.theme.animation_ms, 120);
        assert_eq!(config.terminal.program_status, None);
        assert_eq!(config.terminal.progress, Some(true));
        assert_eq!(config.keys.get("merge").map(String::as_str), Some("M"));
        assert_eq!(
            config.keys.get("refresh").map(String::as_str),
            Some("ctrl-r")
        );
        assert_eq!(
            config.agent.command,
            ["uvx", "--from", "pr-agent", "pr-agent"]
        );
        assert_eq!(config.agent.model, "anthropic/claude-sonnet-5-5");
        assert_eq!(config.agent.fallback_models, ["openai/gpt-6-luna"]);
        assert_eq!(
            config.agent.env,
            BTreeMap::from([
                (
                    "ANTHROPIC__KEY".to_owned(),
                    EnvValue::Command {
                        command: "op read op://dev/anthropic/key".into()
                    }
                ),
                (
                    "OPENAI__KEY".to_owned(),
                    EnvValue::FromEnv {
                        from_env: "OPENAI_API_KEY".into()
                    }
                ),
            ])
        );
        assert_eq!(config.agent.timeout_secs, 600);
        assert_eq!(
            config.agent.extra_instructions.review,
            "Focus on correctness."
        );
        assert_eq!(config.agent.extra_instructions.ask, "");
        assert_eq!(config.action("pr_agent"), None);
        let merge: MergeConfig = toml::Value::Table(config.action("merge").unwrap().clone())
            .try_into()
            .unwrap();
        assert_eq!(merge, config.merge);
        assert_eq!(config.workspaces.root, PathBuf::from("/home/me/work/ws"));
        assert_eq!(
            config.merge_for(&key("pierredotco", "deploy")),
            RepoMerge {
                method: Some(MergeMethodSetting::Rebase),
                bypass_rules: true,
                stack: StackSetting::Auto,
            }
        );
        assert_eq!(
            config.merge_for(&key("chaosteil", "chaoshub")),
            RepoMerge {
                method: Some(MergeMethodSetting::Merge),
                bypass_rules: false,
                stack: StackSetting::Auto,
            }
        );
    }

    #[test]
    fn merge_defaults_without_an_override() {
        let config = Config::parse("", None).unwrap();
        assert_eq!(
            config.merge_for(&key("o", "r")),
            RepoMerge {
                method: None,
                bypass_rules: false,
                stack: StackSetting::Auto,
            }
        );
    }

    /// The `[actions.merge]` table of `text` as the merge action reads it.
    fn merge_action_config(text: &str) -> Option<MergeConfig> {
        let config = Config::parse(text, None).unwrap();
        let table = config.action("merge")?.clone();
        Some(toml::Value::Table(table).try_into().unwrap())
    }

    #[test]
    fn merge_repos_reach_the_merge_action() {
        let text = "[merge.repos.\"o/r\"]\nmethod = \"rebase\"\nbypass_rules = true\n";
        let merge = merge_action_config(text).unwrap();
        assert_eq!(
            merge.repos.get(&key("o", "r")),
            Some(&RepoMerge {
                method: Some(MergeMethodSetting::Rebase),
                bypass_rules: true,
                stack: StackSetting::Auto,
            })
        );
    }

    #[test]
    fn actions_merge_repos_override_merge_repos() {
        let text = concat!(
            "[merge.repos.\"o/r\"]\nmethod = \"rebase\"\n",
            "[merge.repos.\"o/other\"]\nmethod = \"rebase\"\n",
            "[actions.merge.repos.\"o/r\"]\nmethod = \"merge\"\n",
        );
        let merge = merge_action_config(text).unwrap();
        assert_eq!(
            merge.repos,
            BTreeMap::from([(
                key("o", "r"),
                RepoMerge {
                    method: Some(MergeMethodSetting::Merge),
                    bypass_rules: false,
                    stack: StackSetting::Auto,
                }
            )])
        );
    }

    #[test]
    fn merge_keys_are_copied_one_by_one() {
        let text = concat!(
            "[merge]\nmethod = \"squash\"\nbypass_rules = true\n",
            "[actions.merge]\nbypass_rules = false\nother = 1\n",
        );
        let config = Config::parse(text, None).unwrap();
        let table = config.action("merge").unwrap();
        assert_eq!(table["method"].as_str(), Some("squash"));
        assert_eq!(table["bypass_rules"].as_bool(), Some(false));
        assert_eq!(table["other"].as_integer(), Some(1));
        assert!(!table.contains_key("repos"));
    }

    #[test]
    fn merge_stack_reaches_the_merge_action() {
        let text = concat!(
            "[merge]\nstack = \"sequential\"\n",
            "[merge.repos.\"o/r\"]\nstack = \"atomic\"\n",
        );
        let config = Config::parse(text, None).unwrap();
        let table = config.action("merge").unwrap();
        assert_eq!(table["stack"].as_str(), Some("sequential"));
        assert_eq!(config.merge_for(&key("o", "r")).stack, StackSetting::Atomic);
        assert_eq!(
            config.merge_for(&key("o", "other")).stack,
            StackSetting::Auto
        );
        assert!(Config::parse("[merge.repos.\"o/r\"]\nstack = \"fast\"\n", None).is_err());
    }

    #[test]
    fn no_merge_section_adds_no_merge_action_table() {
        assert_eq!(merge_action_config("login = \"me\"\n"), None);
    }

    #[test]
    fn bad_merge_override_is_an_error() {
        let cases = [
            "[merge.repos.chaoshub]\nmethod = \"squash\"\n",
            "[merge.repos.\"o/r\"]\nmethod = \"fast-forward\"\n",
        ];
        for text in cases {
            assert!(Config::parse(text, None).is_err(), "{text}");
        }
    }

    #[test]
    fn bad_repo_name_is_an_error() {
        let err = Config::parse("[[repo]]\nname = \"chaoshub\"\n", None).unwrap_err();
        assert!(err.to_string().contains("owner/name"), "{err}");
    }

    #[test]
    fn load_of_a_missing_file_gives_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = load(&dir.path().join("config.toml")).unwrap();
        assert_eq!(config.refresh, RefreshConfig::default());
    }

    #[test]
    fn load_reports_a_parse_error_with_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "login = [").unwrap();
        let err = load(&path).unwrap_err();
        assert!(matches!(err, ConfigError::Parse { .. }));
        assert!(err.to_string().contains("config.toml"), "{err}");
    }
}
