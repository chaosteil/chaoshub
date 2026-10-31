//! Command line types (clap): `--repo`, `--config`, `--no-prefetch`,
//! `--theme` and `cache clear`.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// A PR tracking TUI for jj, Rex and codeview.
#[derive(Debug, Clone, PartialEq, Eq, Parser)]
#[command(name = "chaoshub", version, about)]
pub struct Cli {
    /// Adds a repo for this session, in the `owner/name` form. Repeat the
    /// option for more repos.
    #[arg(long, value_name = "OWNER/NAME")]
    pub repo: Vec<String>,
    /// Reads the configuration from this file instead of
    /// `~/.config/chaoshub/config.toml`.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,
    /// Turns off the background fetch of pull request refs for this
    /// session.
    #[arg(long)]
    pub no_prefetch: bool,
    /// Uses this theme style for this session, in place of `[theme]
    /// style`.
    #[arg(long, value_name = "STYLE")]
    pub theme: Option<ThemeArg>,
    #[command(subcommand)]
    pub command: Option<CacheCommand>,
}

/// A theme style on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ThemeArg {
    /// Rough block borders and textures.
    Grunge,
    /// Black, white and hard red with angular cuts.
    Punk,
    /// Rounded borders, no textures.
    Plain,
}

/// The subcommands. Without one, chaoshub starts the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
pub enum CacheCommand {
    /// Manages the SQLite cache.
    #[command(subcommand)]
    Cache(CacheAction),
}

/// The `cache` subcommands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Subcommand)]
pub enum CacheAction {
    /// Deletes the cache file. The next start syncs from GitHub again.
    Clear,
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("chaoshub").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn the_command_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn no_arguments_start_the_tui() {
        assert_eq!(
            parse(&[]),
            Cli {
                repo: Vec::new(),
                config: None,
                no_prefetch: false,
                theme: None,
                command: None,
            }
        );
    }

    #[test]
    fn theme_parses() {
        assert_eq!(parse(&["--theme", "punk"]).theme, Some(ThemeArg::Punk));
        assert_eq!(parse(&["--theme", "plain"]).theme, Some(ThemeArg::Plain));
        assert_eq!(parse(&["--theme", "grunge"]).theme, Some(ThemeArg::Grunge));
        assert!(Cli::try_parse_from(["chaoshub", "--theme", "neon"]).is_err());
    }

    #[test]
    fn options_parse() {
        assert_eq!(
            parse(&[
                "--repo",
                "chaosteil/chaoshub",
                "--repo",
                "pierredotco/deploy",
                "--config",
                "/tmp/c.toml",
                "--no-prefetch",
            ]),
            Cli {
                repo: vec!["chaosteil/chaoshub".into(), "pierredotco/deploy".into()],
                config: Some(PathBuf::from("/tmp/c.toml")),
                no_prefetch: true,
                theme: None,
                command: None,
            }
        );
    }

    #[test]
    fn cache_clear_parses() {
        assert_eq!(
            parse(&["cache", "clear"]).command,
            Some(CacheCommand::Cache(CacheAction::Clear))
        );
        assert!(Cli::try_parse_from(["chaoshub", "cache"]).is_err());
    }
}
