//! chaoshub: a PR tracking TUI for GitHub, with jj, Rex and codeview support.
//!
//! All logic lives in the modules of this library. The binary in `main.rs`
//! is thin.
//!
//! The dependency rule. Module dependencies point inward, toward `domain`:
//!
//! ```text
//! domain <- bus <- github | vcs | store | launch <- workers | actions <- app <- ui <- run
//! config: read by app, workers, actions, agent, run.   theme: read by ui and run.   cli: read by main and run.
//! agent: reads domain, bus, github and config; read by actions, app and run.
//! codeview: reads domain and config; read by actions, app (exec) and run.
//! osc: reads bus only; read by run and clipboard.   clipboard: read by app (exec).
//! markup: reads no module; read by app and ui.
//! ```
#![forbid(unsafe_code)]

mod actions;
mod agent;
mod app;
mod bus;
pub mod cli;
mod clipboard;
mod codeview;
mod config;
mod domain;
mod github;
mod launch;
mod markup;
mod osc;
mod run;
mod store;
mod theme;
mod ui;
mod vcs;
mod workers;

pub use config::paths::state_dir;
pub use theme::ThemeKind;

/// Runs the command of `cli`: starts the TUI and runs it until the user
/// quits, or runs `cache clear`. `theme_kind` is the color support of the
/// terminal.
pub fn run(cli: cli::Cli, theme_kind: ThemeKind) -> anyhow::Result<()> {
    run::run(cli, theme_kind)
}

/// Entry points for the live tests in `tests/live.rs`. The GitHub client
/// and its calls are `pub(crate)`, so this module wraps the calls that the
/// live tests use in `pub` items. It is not a stable API.
#[doc(hidden)]
pub mod testing {
    use crate::bus::MergeProgress;
    use crate::domain::keys::{PrKey, RepoKey};
    use crate::github::parse::Me;
    use crate::github::rest::MergeArgs;
    use crate::github::{GITHUB_API, GithubClient, TokenSource};

    /// The crate's GitHub client on github.com, with the token of
    /// `gh auth token`.
    #[derive(Debug)]
    pub struct LiveClient {
        client: GithubClient,
    }

    /// The fields of a pull request that the GraphQL client reads.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct LivePr {
        pub number: u64,
        pub title: String,
        pub body: String,
        /// `Open`, `Closed` or `Merged`.
        pub state: String,
        pub head_oid: String,
        pub base_ref: String,
        /// The `mergeStateStatus`, for example `Clean` or `Unknown`.
        pub merge_state: String,
        /// The `mergeable` value, for example `Mergeable` or `Unknown`.
        pub mergeable: String,
        /// `None` when the pull request is not in a stack.
        pub stack_size: Option<u32>,
        /// `None` when the pull request is not in a stack.
        pub stack_position: Option<u32>,
    }

    /// The state of an async merge.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum LiveMergeProgress {
        Pending,
        Merged { sha: String },
        Enqueued,
        Failed(String),
    }

    impl From<MergeProgress> for LiveMergeProgress {
        fn from(progress: MergeProgress) -> Self {
            match progress {
                MergeProgress::Pending => Self::Pending,
                MergeProgress::Merged { sha } => Self::Merged { sha },
                MergeProgress::Enqueued => Self::Enqueued,
                MergeProgress::Failed(message) => Self::Failed(message),
            }
        }
    }

    /// The pull request `number` of `owner/name`.
    fn key(owner: &str, name: &str, number: u64) -> PrKey {
        PrKey {
            repo: RepoKey {
                owner: owner.to_owned(),
                name: name.to_owned(),
            },
            number,
        }
    }

    impl LiveClient {
        /// A client for github.com that runs `gh auth token`.
        pub fn gh() -> anyhow::Result<Self> {
            Ok(Self {
                client: GithubClient::new(TokenSource::gh(), GITHUB_API)?,
            })
        }

        /// The login of the token owner, from `viewer { login }`.
        pub async fn viewer_login(&self) -> anyhow::Result<String> {
            Ok(self.client.viewer().await?.0)
        }

        /// The pull request with the GraphQL node id `node_id`, read for the
        /// user `login`. `None` when the id names no pull request.
        pub async fn pr_by_node_id(
            &self,
            node_id: &str,
            login: &str,
        ) -> anyhow::Result<Option<LivePr>> {
            let me = Me { login, teams: &[] };
            let (prs, _) = self.client.pr_by_ids(&[node_id.to_owned()], &me).await?;
            Ok(prs.into_iter().next().map(|pr| {
                let head = pr.head.value;
                LivePr {
                    number: pr.key.number,
                    title: head.title,
                    body: head.body,
                    state: format!("{:?}", head.state),
                    head_oid: head.head_oid,
                    base_ref: head.base_ref,
                    merge_state: format!("{:?}", head.merge_state),
                    mergeable: format!("{:?}", head.mergeable),
                    stack_size: head.stack.as_ref().map(|stack| stack.size),
                    stack_position: head.stack.as_ref().map(|stack| stack.position),
                }
            }))
        }

        /// Requests an async merge of the pull request `number` of
        /// `owner/name` with `merge_action: "default"` and no rule bypass.
        /// It returns the uuid to poll and the first progress.
        pub async fn merge_async(
            &self,
            (owner, name, number): (&str, &str, u64),
            method: &str,
            commit_title: &str,
            commit_message: &str,
            sha: &str,
        ) -> anyhow::Result<(Option<String>, LiveMergeProgress)> {
            let args = MergeArgs {
                method,
                commit_title,
                commit_message,
                sha,
                bypass_rules: false,
            };
            let started = self
                .client
                .merge_async(&key(owner, name, number), &args)
                .await?;
            Ok((started.uuid, started.progress.into()))
        }

        /// The state of the async merge `uuid` of the pull request `number`
        /// of `owner/name`.
        pub async fn merge_async_status(
            &self,
            (owner, name, number): (&str, &str, u64),
            uuid: &str,
        ) -> anyhow::Result<LiveMergeProgress> {
            let progress = self
                .client
                .merge_async_status(&key(owner, name, number), uuid)
                .await?;
            Ok(progress.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    /// For each top-level module, the modules that it can import with
    /// `crate::`. A module can always import itself.
    const ALLOWED: &[(&str, &[&str])] = &[
        ("domain", &[]),
        ("bus", &["domain"]),
        ("github", &["domain", "bus"]),
        ("vcs", &["domain", "bus"]),
        ("store", &["domain", "bus"]),
        ("launch", &["domain", "bus"]),
        ("agent", &["domain", "bus", "github", "config"]),
        ("codeview", &["domain", "config"]),
        (
            "workers",
            &[
                "domain", "bus", "github", "vcs", "store", "launch", "config",
            ],
        ),
        (
            "actions",
            &[
                "domain", "bus", "github", "vcs", "store", "launch", "config", "agent", "codeview",
            ],
        ),
        (
            "app",
            &[
                "domain",
                "bus",
                "github",
                "vcs",
                "store",
                "launch",
                "workers",
                "actions",
                "agent",
                "codeview",
                "config",
                "clipboard",
                "markup",
            ],
        ),
        ("ui", &["domain", "bus", "app", "theme", "markup"]),
        (
            "run",
            &[
                "domain",
                "bus",
                "github",
                "vcs",
                "store",
                "launch",
                "workers",
                "actions",
                "agent",
                "codeview",
                "app",
                "ui",
                "config",
                "theme",
                "osc",
                "clipboard",
                "cli",
            ],
        ),
        ("markup", &[]),
        ("config", &["domain"]),
        ("theme", &[]),
        ("osc", &["bus"]),
        ("clipboard", &["osc"]),
        ("cli", &[]),
    ];

    /// For each top-level module, the external crates that it must not
    /// name. `app` reads the crossterm key types through `bus`.
    const FORBIDDEN_CRATES: &[(&str, &[&str])] = &[("app", &["crossterm"])];

    /// Adds every `.rs` file under `dir` to `out`.
    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                rust_files(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }

    /// The top-level module of a file path relative to `src`. `None` for
    /// the crate roots `lib.rs` and `main.rs`.
    fn owner(rel: &Path) -> Option<String> {
        let first = rel.components().next()?.as_os_str().to_str()?;
        match first {
            "lib.rs" | "main.rs" => None,
            name => Some(name.trim_end_matches(".rs").to_owned()),
        }
    }

    /// The identifier at the start of `s`.
    fn ident(s: &str) -> String {
        s.chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect()
    }

    /// The first identifier of each top-level item in a `{a, b::{c}}`
    /// group. `group` starts after the opening brace.
    fn group_heads(group: &str) -> Vec<String> {
        let mut heads = Vec::new();
        let mut depth = 0usize;
        let mut at_start = true;
        for (i, c) in group.char_indices() {
            match c {
                '{' => depth += 1,
                '}' if depth == 0 => break,
                '}' => depth -= 1,
                ',' if depth == 0 => at_start = true,
                c if at_start && (c.is_alphanumeric() || c == '_') => {
                    heads.push(ident(&group[i..]));
                    at_start = false;
                }
                _ => {}
            }
        }
        heads
    }

    /// The module names after every `crate::` token in `src`, including
    /// the items of `crate::{a, b}` groups.
    fn crate_refs(src: &str) -> Vec<String> {
        const TOKEN: &str = "crate::";
        let mut refs = Vec::new();
        let mut rest = src;
        while let Some(i) = rest.find(TOKEN) {
            let before = rest[..i].chars().next_back();
            rest = &rest[i + TOKEN.len()..];
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '$') {
                continue;
            }
            match rest.strip_prefix('{') {
                Some(group) => refs.extend(group_heads(group)),
                None => refs.push(ident(rest)),
            }
        }
        refs
    }

    /// True when `src` has a `<krate>::` path that is not the tail of a
    /// longer identifier.
    fn names_crate(src: &str, krate: &str) -> bool {
        let token = format!("{krate}::");
        src.match_indices(&token).any(|(i, _)| {
            !src[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
        })
    }

    #[test]
    fn names_crate_finds_paths_only() {
        assert!(names_crate("use crossterm::event::KeyEvent;", "crossterm"));
        assert!(names_crate("x: ::crossterm::event::KeyCode", "crossterm"));
        assert!(!names_crate("use my_crossterm::x;", "crossterm"));
        assert!(!names_crate("// the crossterm key types", "crossterm"));
    }

    #[test]
    fn crate_refs_reads_paths_and_groups() {
        let src = "use crate::domain::keys::PrKey;\n\
                   use crate::{bus::{Msg, Cmd}, theme};\n\
                   $crate::x!(); pub(crate) fn f() {}";
        assert_eq!(crate_refs(src), ["domain", "bus", "theme"]);
    }

    #[test]
    fn modules_follow_the_dependency_rule() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        let mut errors = Vec::new();
        for file in &files {
            let rel = file.strip_prefix(&src).unwrap();
            let Some(owner) = owner(rel) else { continue };
            let Some((_, allowed)) = ALLOWED.iter().find(|(name, _)| *name == owner) else {
                errors.push(format!(
                    "{}: module `{owner}` is not in the allow matrix",
                    rel.display()
                ));
                continue;
            };
            let text = std::fs::read_to_string(file).unwrap();
            let forbidden = FORBIDDEN_CRATES
                .iter()
                .filter(|(name, _)| *name == owner)
                .flat_map(|(_, crates)| crates.iter());
            for krate in forbidden {
                if names_crate(&text, krate) {
                    errors.push(format!(
                        "{}: `{owner}` must not import `{krate}`",
                        rel.display()
                    ));
                }
            }
            for used in crate_refs(&text) {
                if used != owner && !allowed.contains(&used.as_str()) {
                    errors.push(format!(
                        "{}: `{owner}` must not import `crate::{used}`",
                        rel.display()
                    ));
                }
            }
        }
        assert!(
            errors.is_empty(),
            "layering violations:\n{}",
            errors.join("\n")
        );
    }
}
