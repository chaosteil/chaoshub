//! The syntax highlight of the code lines of a diff, with syntect and the
//! syntaxes of bat that two-face bundles. The result is a role per token,
//! not a color: the palette of the theme gives the colors (see
//! `ui::style`).
//!
//! The highlight is slow for a large patch, so only the highlight worker
//! calls it, on a blocking thread (see `workers::highlight`).

use std::str::FromStr;
use std::sync::LazyLock;

use syntect::easy::HighlightLines;
use syntect::highlighting::{
    Color, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings,
};
use syntect::parsing::{SyntaxReference, SyntaxSet};

use crate::domain::diff::{LineKind, Syntax, Tokens, code, numbered};

/// A patch with more bytes than this gets no highlight, so one file
/// cannot keep the worker busy for long. At this limit, a release build
/// takes at most about 400 ms, for a patch of context lines only.
const MAX_BYTES: usize = 128 * 1024;

/// A line longer than this gets no syntax colors and does not go through
/// the parser. A long line, as in minified code, makes the parser slow.
const MAX_LINE: usize = 1000;

/// The side of a diff that a code line is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    /// A removed line: only in the old file.
    Old,
    /// An added line: only in the new file.
    New,
    /// A context line: in both files.
    Both,
}

/// The syntaxes of bat that two-face bundles, loaded at the first
/// highlight.
static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);

/// The theme that gives each scope its role. The scopes follow the
/// `pierre-dark` theme.
static ROLES: LazyLock<Theme> = LazyLock::new(roles);

/// The scopes of each role. A more specific scope wins over a less
/// specific one, so `constant.numeric` is a number and not a constant.
const SCOPES: &[(&str, Syntax)] = &[
    ("comment, punctuation.definition.comment", Syntax::Comment),
    (
        "string, constant.other.symbol, punctuation.definition.string",
        Syntax::String,
    ),
    ("constant.numeric, constant.language", Syntax::Number),
    (
        "constant, variable.other.constant, support.constant",
        Syntax::Constant,
    ),
    ("keyword, storage", Syntax::Keyword),
    ("keyword.operator", Syntax::Operator),
    (
        "entity.name.function, support.function, variable.function",
        Syntax::Function,
    ),
    (
        "entity.name.type, entity.name.class, entity.name.struct, entity.name.enum, \
         entity.name.trait, entity.name.interface, support.type, support.class, \
         entity.other.inherited-class",
        Syntax::Type,
    ),
    (
        "entity.name.namespace, entity.name.module, support.module",
        Syntax::Namespace,
    ),
    ("variable", Syntax::Variable),
    ("variable.parameter", Syntax::Parameter),
    ("punctuation", Syntax::Punctuation),
];

/// All roles, by their index in the theme colors.
const ALL: [Syntax; 13] = [
    Syntax::Plain,
    Syntax::Comment,
    Syntax::String,
    Syntax::Number,
    Syntax::Constant,
    Syntax::Keyword,
    Syntax::Operator,
    Syntax::Function,
    Syntax::Type,
    Syntax::Namespace,
    Syntax::Variable,
    Syntax::Parameter,
    Syntax::Punctuation,
];

/// The theme color that stands for `role`: the red channel holds the
/// index of the role.
fn marker(role: Syntax) -> Color {
    let index = ALL.iter().position(|&r| r == role).unwrap_or_default();
    Color {
        r: u8::try_from(index).unwrap_or_default(),
        g: 0,
        b: 0,
        a: 0xff,
    }
}

/// The role of a theme color that `marker` made.
fn role(color: Color) -> Syntax {
    ALL.get(usize::from(color.r))
        .copied()
        .unwrap_or(Syntax::Plain)
}

/// The theme of `SCOPES`, with `Plain` as the default.
fn roles() -> Theme {
    let scopes = SCOPES
        .iter()
        .filter_map(|&(selectors, role)| {
            Some(ThemeItem {
                scope: ScopeSelectors::from_str(selectors).ok()?,
                style: StyleModifier {
                    foreground: Some(marker(role)),
                    background: None,
                    font_style: None,
                },
            })
        })
        .collect();
    Theme {
        settings: ThemeSettings {
            foreground: Some(marker(Syntax::Plain)),
            ..ThemeSettings::default()
        },
        scopes,
        ..Theme::default()
    }
}

/// File extensions that the syntaxes do not know, each with an extension
/// of the same language that they know.
const EXTENSIONS: &[(&str, &str)] = &[
    // The TypeScript syntax of JSX also reads the JSX tags.
    ("jsx", "tsx"),
    ("mjs", "js"),
    ("cjs", "js"),
    ("jsonc", "json"),
    ("json5", "json"),
];

/// File names that the syntaxes do not know, each with an extension of
/// their language.
const NAMES: &[(&str, &str)] = &[("flake.lock", "json"), ("Containerfile", "Dockerfile")];

/// The syntax of a file at `path`, by its file name or its extension.
/// `None` for a file that no bundled syntax knows.
fn syntax_for(path: &str) -> Option<&'static SyntaxReference> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let syntaxes = &*SYNTAXES;
    let named = NAMES
        .iter()
        .find_map(|&(known, ext)| (known == name).then_some(ext));
    // `Dockerfile.dev` and `Containerfile.prod` are Dockerfiles too.
    let docker = ["Dockerfile.", "Containerfile."]
        .iter()
        .any(|prefix| name.starts_with(prefix))
        .then_some("Dockerfile");
    let extension = name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase());
    let alias = extension.as_deref().and_then(|ext| {
        EXTENSIONS
            .iter()
            .find_map(|&(known, other)| (known == ext).then_some(other))
    });
    let found = [Some(name), named, docker, extension.as_deref(), alias]
        .into_iter()
        .flatten()
        .find_map(|key| syntaxes.find_syntax_by_extension(key))?;
    (found.name != "Plain Text").then_some(found)
}

/// Loads the syntax set, so the first highlight does not wait for it.
pub(crate) fn warm() {
    LazyLock::force(&SYNTAXES);
}

/// The tokens of each code line of `text`, a patch of the file at
/// `path`: one entry per added, removed and context line, in patch
/// order. `None` as for `highlight`.
pub(crate) fn patch(path: &str, text: &str) -> Option<Vec<Tokens>> {
    let lines: Vec<(Side, &str)> = numbered(text)
        .into_iter()
        .filter_map(|(kind, _, text)| {
            let side = match kind {
                LineKind::Added => Side::New,
                LineKind::Removed => Side::Old,
                LineKind::Context => Side::Both,
                LineKind::Hunk | LineKind::Note => return None,
            };
            Some((side, code(text)))
        })
        .collect();
    highlight(path, &lines)
}

/// The tokens of each code line of a diff of the file at `path`. Each
/// line is its side and its text without the `+`, `-` or space prefix.
///
/// The old and the new file each get their own parser, so a removed
/// line does not change the parse of an added line. A context line goes
/// through both, and its tokens come from the new file. A line over
/// `MAX_LINE` is one `Plain` token.
///
/// `None` for a language without a bundled syntax, a patch over the size
/// limit, or a parse error.
pub(crate) fn highlight(path: &str, lines: &[(Side, &str)]) -> Option<Vec<Tokens>> {
    let bytes: usize = lines.iter().map(|(_, text)| text.len() + 1).sum();
    if bytes > MAX_BYTES {
        return None;
    }
    let syntax = syntax_for(path)?;
    let syntaxes = &*SYNTAXES;
    let mut old = HighlightLines::new(syntax, &ROLES);
    let mut new = HighlightLines::new(syntax, &ROLES);
    let mut out = Vec::with_capacity(lines.len());
    for &(side, text) in lines {
        if text.len() > MAX_LINE {
            out.push(vec![(Syntax::Plain, text.to_owned())]);
            continue;
        }
        let line = format!("{text}\n");
        if side == Side::Both {
            old.highlight_line(&line, syntaxes).ok()?;
        }
        let parser = if side == Side::Old {
            &mut old
        } else {
            &mut new
        };
        let ranges = parser.highlight_line(&line, syntaxes).ok()?;
        let mut tokens: Tokens = Vec::new();
        for (style, piece) in ranges {
            let piece = piece.strip_suffix('\n').unwrap_or(piece);
            if piece.is_empty() {
                continue;
            }
            let role = role(style.foreground);
            match tokens.last_mut() {
                Some((last, joined)) if *last == role => joined.push_str(piece),
                _ => tokens.push((role, piece.to_owned())),
            }
        }
        out.push(tokens);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The role of the first token with `text` in `tokens`.
    fn role_of(tokens: &Tokens, text: &str) -> Syntax {
        tokens
            .iter()
            .find_map(|(role, t)| t.contains(text).then_some(*role))
            .unwrap_or_else(|| panic!("no {text:?} in {tokens:?}"))
    }

    #[test]
    fn rust_tokens_get_their_roles() {
        let lines = [
            (Side::Both, "fn main() {"),
            (Side::New, "    let n = 42; // the answer"),
            (Side::New, "    println!(\"hi {n}\");"),
        ];
        let out = highlight("src/main.rs", &lines).unwrap();
        assert_eq!(role_of(&out[0], "fn"), Syntax::Keyword);
        assert_eq!(role_of(&out[0], "main"), Syntax::Function);
        assert_eq!(role_of(&out[1], "let"), Syntax::Keyword);
        assert_eq!(role_of(&out[1], "42"), Syntax::Number);
        assert_eq!(role_of(&out[1], "the answer"), Syntax::Comment);
        assert_eq!(role_of(&out[2], "hi"), Syntax::String);
        assert_eq!(role_of(&out[2], "{n}"), Syntax::Constant, "a placeholder");
        let text: Vec<String> = out
            .iter()
            .map(|tokens| tokens.iter().map(|(_, t)| t.as_str()).collect())
            .collect();
        assert_eq!(
            text,
            lines.map(|(_, t)| t.to_owned()),
            "the tokens keep all the text"
        );
    }

    #[test]
    fn typescript_toml_nix_and_dockerfile_get_roles() {
        let ts = highlight("web/app.ts", &[(Side::New, "const n: number = 42;")]).unwrap();
        assert_eq!(role_of(&ts[0], "const"), Syntax::Keyword);
        assert_eq!(role_of(&ts[0], "42"), Syntax::Number);
        let toml = highlight("Cargo.toml", &[(Side::New, "name = \"chaoshub\"")]).unwrap();
        assert_eq!(role_of(&toml[0], "chaoshub"), Syntax::String);
        let nix = highlight("flake.nix", &[(Side::New, "  x = \"y\"; # note")]).unwrap();
        assert_eq!(role_of(&nix[0], "note"), Syntax::Comment);
        let docker = highlight("Dockerfile", &[(Side::New, "FROM rust:1.91")]).unwrap();
        assert_eq!(role_of(&docker[0], "FROM"), Syntax::Keyword);
    }

    #[test]
    fn a_removed_line_does_not_change_the_new_side() {
        let lines = [(Side::Old, "/* an open comment"), (Side::New, "let x = 1;")];
        let out = highlight("a.rs", &lines).unwrap();
        assert_eq!(role_of(&out[0], "open"), Syntax::Comment);
        assert_eq!(role_of(&out[1], "let"), Syntax::Keyword);
    }

    #[test]
    fn the_file_name_or_the_extension_picks_the_syntax() {
        let name = |path| syntax_for(path).map(|s| s.name.as_str());
        assert_eq!(name("src/lib.rs"), Some("Rust"));
        assert_eq!(name("build/Makefile"), Some("Makefile"));
        assert_eq!(name("web/app.ts"), Some("TypeScript"));
        assert_eq!(name("web/App.tsx"), Some("TypeScriptReact"));
        assert_eq!(name("web/App.jsx"), Some("TypeScriptReact"));
        assert_eq!(name("web/main.mjs"), Some("JavaScript"));
        assert_eq!(name("Cargo.toml"), Some("TOML"));
        assert_eq!(name("Cargo.lock"), Some("TOML"));
        assert_eq!(name("flake.nix"), Some("Nix"));
        assert_eq!(name("flake.lock"), Some("JSON"));
        assert_eq!(name("Dockerfile"), Some("Dockerfile"));
        assert_eq!(name("ci/Dockerfile.dev"), Some("Dockerfile"));
        assert_eq!(name("Containerfile"), Some("Dockerfile"));
        assert_eq!(
            name("src/MAIN.RS"),
            Some("Rust"),
            "the extension in any case"
        );
        assert_eq!(name("go.mod"), Some("Gomod"));
        assert_eq!(name("notes.txt"), None, "plain text has no colors");
        assert_eq!(name("data.unknown"), None);
        assert_eq!(name("LICENSE"), None);
    }

    #[test]
    fn an_unknown_language_or_a_large_patch_gets_no_highlight() {
        assert_eq!(highlight("a.unknown", &[(Side::New, "x")]), None);
        let line = "let x = 1;".repeat(50);
        let many = vec![(Side::New, line.as_str()); MAX_BYTES / line.len() + 1];
        assert_eq!(highlight("a.rs", &many), None);
    }

    #[test]
    fn a_long_line_is_plain_and_the_next_lines_keep_their_roles() {
        let long = format!("let s = \"{}", "x".repeat(MAX_LINE));
        let lines = [(Side::New, long.as_str()), (Side::New, "let n = 1;")];
        let out = highlight("a.rs", &lines).unwrap();
        assert_eq!(out[0], [(Syntax::Plain, long.clone())]);
        assert_eq!(
            role_of(&out[1], "let"),
            Syntax::Keyword,
            "the open string of the long line does not reach the parser"
        );
    }

    #[test]
    fn a_patch_gives_one_entry_per_code_line() {
        let patch =
            "@@ -1,2 +1,2 @@\n fn a() {}\n-let x = 1;\n+let y = 2;\n\\ No newline at end of file";
        let out = super::patch("a.rs", patch).unwrap();
        assert_eq!(out.len(), 3, "no entry for the hunk header and the note");
        assert_eq!(role_of(&out[0], "fn"), Syntax::Keyword);
        assert_eq!(role_of(&out[2], "2"), Syntax::Number);
    }
}
