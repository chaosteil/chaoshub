//! The syntax highlight of the code lines of a diff, with syntect and the
//! syntaxes that syntect bundles. The result is a role per token, not a
//! color: the palette of the theme gives the colors (see `ui::style`).

use std::str::FromStr;
use std::sync::LazyLock;

use syntect::easy::HighlightLines;
use syntect::highlighting::{
    Color, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSettings,
};
use syntect::parsing::{SyntaxReference, SyntaxSet};

use crate::app::models::Syntax;

/// A patch with more bytes than this gets no highlight, so the first
/// draw of a large file stays fast. The highlight runs on the UI thread
/// when a file opens. At this limit, a release build takes at most about
/// 120 ms.
const MAX_BYTES: usize = 32 * 1024;

/// A patch with a line longer than this gets no highlight. A long line,
/// as in minified code, makes the parser slow.
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

/// The tokens of one code line, each with its role. Two tokens next to
/// each other have different roles.
pub(crate) type Tokens = Vec<(Syntax, String)>;

/// The syntaxes that syntect bundles, loaded at the first highlight.
static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);

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

/// The syntax of a file at `path`, by its file name or its extension.
/// `None` for a file that no bundled syntax knows.
fn syntax_for(path: &str) -> Option<&'static SyntaxReference> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let extension = name.rsplit_once('.').map(|(_, ext)| ext);
    let syntaxes = &*SYNTAXES;
    let found = syntaxes
        .find_syntax_by_extension(name)
        .or_else(|| extension.and_then(|ext| syntaxes.find_syntax_by_extension(ext)))?;
    (found.name != "Plain Text").then_some(found)
}

/// The tokens of each code line of a diff of the file at `path`. Each
/// line is its side and its text without the `+`, `-` or space prefix.
///
/// The old and the new file each get their own parser, so a removed
/// line does not change the parse of an added line. A context line goes
/// through both, and its tokens come from the new file.
///
/// `None` for a language without a bundled syntax, a patch over the size
/// limits, or a parse error.
pub(crate) fn highlight(path: &str, lines: &[(Side, &str)]) -> Option<Vec<Tokens>> {
    let bytes: usize = lines.iter().map(|(_, text)| text.len() + 1).sum();
    if bytes > MAX_BYTES || lines.iter().any(|(_, text)| text.len() > MAX_LINE) {
        return None;
    }
    let syntax = syntax_for(path)?;
    let syntaxes = &*SYNTAXES;
    let mut old = HighlightLines::new(syntax, &ROLES);
    let mut new = HighlightLines::new(syntax, &ROLES);
    let mut out = Vec::with_capacity(lines.len());
    for &(side, text) in lines {
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
        assert_eq!(name("notes.txt"), None, "plain text has no colors");
        assert_eq!(name("data.unknown"), None);
        assert_eq!(name("LICENSE"), None);
    }

    #[test]
    fn an_unknown_language_or_a_large_patch_gets_no_highlight() {
        assert_eq!(highlight("a.unknown", &[(Side::New, "x")]), None);
        let long = "x".repeat(MAX_LINE + 1);
        assert_eq!(highlight("a.rs", &[(Side::New, long.as_str())]), None);
        let line = "let x = 1;".repeat(50);
        let many = vec![(Side::New, line.as_str()); MAX_BYTES / line.len() + 1];
        assert_eq!(highlight("a.rs", &many), None);
    }
}
