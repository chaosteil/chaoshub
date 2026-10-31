//! Command line types (clap): `--repo`, `--config`, `--no-prefetch`,
//! `--theme`, `--palette`, `cache clear` and `pr OWNER/NAME NUMBER`.

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
    /// Uses this palette for this session, in place of `[theme] palette`.
    #[arg(long, value_name = "NAME")]
    pub palette: Option<PaletteArg>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// A theme style on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ThemeArg {
    /// Rough block borders and textures.
    Grunge,
    /// Angular cuts, diagonal accents, inverted headers and a selection
    /// band.
    Punk,
    /// Rounded borders, no textures.
    Plain,
}

/// A palette on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PaletteArg {
    /// The Pierre colors. A 16-color terminal gets the ANSI colors.
    Pierre,
    /// The 16 ANSI colors.
    Ansi16,
}

/// The subcommands. Without one, chaoshub starts the TUI.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Manages the SQLite cache.
    #[command(subcommand)]
    Cache(CacheAction),
    /// Shows one pull request. The detail pane of the list runs it.
    Pr {
        /// The repo of the pull request, in the `owner/name` form.
        #[arg(value_name = "OWNER/NAME")]
        repo: String,
        /// The number of the pull request.
        number: u64,
    },
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
                palette: None,
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
    fn palette_parses() {
        assert_eq!(
            parse(&["--palette", "ansi16"]).palette,
            Some(PaletteArg::Ansi16)
        );
        assert_eq!(
            parse(&["--palette", "pierre"]).palette,
            Some(PaletteArg::Pierre)
        );
        assert!(Cli::try_parse_from(["chaoshub", "--palette", "neon"]).is_err());
    }

    #[test]
    fn options_parse() {
        assert_eq!(
            parse(&[
                "--repo",
                "chaosteil/chaoshub",
                "--repo",
                "acme/deploy",
                "--config",
                "/tmp/c.toml",
                "--no-prefetch",
            ]),
            Cli {
                repo: vec!["chaosteil/chaoshub".into(), "acme/deploy".into()],
                config: Some(PathBuf::from("/tmp/c.toml")),
                no_prefetch: true,
                theme: None,
                palette: None,
                command: None,
            }
        );
    }

    #[test]
    fn cache_clear_parses() {
        assert_eq!(
            parse(&["cache", "clear"]).command,
            Some(Command::Cache(CacheAction::Clear))
        );
        assert!(Cli::try_parse_from(["chaoshub", "cache"]).is_err());
    }

    #[test]
    fn pr_parses() {
        assert_eq!(
            parse(&["pr", "chaosteil/chaoshub", "7"]).command,
            Some(Command::Pr {
                repo: "chaosteil/chaoshub".into(),
                number: 7,
            })
        );
        assert!(Cli::try_parse_from(["chaoshub", "pr", "chaosteil/chaoshub"]).is_err());
        assert!(Cli::try_parse_from(["chaoshub", "pr"]).is_err());
    }
}
