//! chaoshub: a PR tracking TUI for jj, Rex and codeview.
//!
//! All logic lives in the modules of this library. The binary in `main.rs`
//! is thin.
//!
//! The dependency rule. Module dependencies point inward, toward `domain`:
//!
//! ```text
//! domain <- bus <- github | vcs | store | launch <- workers | actions <- app <- ui <- run
//! config: read by app, workers, actions, run.   theme: read by ui and run.   cli: read by main only.
//! osc: reads bus only; read by run and clipboard.   clipboard: read by app (exec).
//! ```
#![forbid(unsafe_code)]

mod actions;
mod app;
mod bus;
pub mod cli;
mod clipboard;
mod config;
mod domain;
mod github;
mod launch;
mod osc;
mod run;
mod store;
mod theme;
mod ui;
mod vcs;
mod workers;

pub use config::paths::state_dir;

/// Starts the TUI and runs it until the user quits.
pub fn run() -> anyhow::Result<()> {
    run::run()
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
        (
            "workers",
            &[
                "domain", "bus", "github", "vcs", "store", "launch", "config",
            ],
        ),
        (
            "actions",
            &[
                "domain", "bus", "github", "vcs", "store", "launch", "config",
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
                "config",
                "clipboard",
            ],
        ),
        ("ui", &["domain", "bus", "app", "theme"]),
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
                "app",
                "ui",
                "config",
                "theme",
                "osc",
                "clipboard",
            ],
        ),
        ("config", &["domain"]),
        ("theme", &[]),
        ("osc", &["bus"]),
        ("clipboard", &["osc"]),
        ("cli", &[]),
    ];

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
