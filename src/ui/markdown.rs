//! Draws comment bodies in the theme colors.
//!
//! `markup::rows` renders the HTML of GitHub, or local markdown, and wraps
//! it to the width. This module turns the marks of each piece into the
//! theme colors: headings, emphasis, links, quotes, images, table borders,
//! and code spans and blocks in the code colors.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::markup::{self, Marks, Source};
use crate::theme::Palette;

/// The style of a piece with `marks`.
fn style(marks: Marks, palette: &Palette) -> Style {
    let mut style = Style::new().fg(palette.fg);
    if marks.image || marks.quote || marks.border {
        style = style.fg(palette.fg_mute);
    }
    if marks.image {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if marks.heading {
        style = style.fg(palette.primary).add_modifier(Modifier::BOLD);
    }
    if marks.link {
        style = style.fg(palette.info).add_modifier(Modifier::UNDERLINED);
    }
    if marks.code || marks.pre {
        style = style.fg(palette.orange).bg(palette.surface);
    }
    if marks.strong {
        style = style.add_modifier(Modifier::BOLD);
    }
    if marks.emphasis {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if marks.strike {
        style = style.add_modifier(Modifier::CROSSED_OUT);
    }
    style
}

/// Renders a body to lines of at most `width` cells. Only lines of
/// preformatted text can be wider.
pub(crate) fn render(source: Source<'_>, width: u16, palette: &Palette) -> Vec<Line<'static>> {
    markup::rows(source, usize::from(width.max(1)))
        .iter()
        .map(|row| {
            Line::from(
                row.iter()
                    .map(|piece| Span::styled(piece.text.clone(), style(piece.marks, palette)))
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(ToString::to_string).collect()
    }

    fn span<'a>(lines: &'a [Line<'_>], word: &str) -> &'a Span<'a> {
        lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content.contains(word))
            .unwrap()
    }

    #[test]
    fn markdown_body() {
        let body = "## Review\n\nThe `Transitions::apply` call must run **before** `prepare`.\n\n\
                    - one\n- two\n\n> quoted text\n\n```rust\nlet x = 1;\n```";
        let lines = render(Source::Markdown(body), 40, &Palette::pierre());
        insta::assert_snapshot!(texts(&lines).join("\n"));
        assert!(lines.iter().all(|l| l.width() <= 40));
    }

    #[test]
    fn code_spans_use_the_code_colors() {
        let p = Palette::pierre();
        let lines = render(Source::Markdown("call `apply` first"), 40, &p);
        let code = span(&lines, "apply");
        assert_eq!(code.style.fg, Some(p.orange));
        assert_eq!(code.style.bg, Some(p.surface));
    }

    #[test]
    fn marks_use_the_theme_colors() {
        let p = Palette::pierre();
        let html = "<h2>Title</h2><p><a href=\"x\">link</a> <del>gone</del> \
                    <img alt=\"shot\" src=\"y\"></p><blockquote><p>said</p></blockquote>";
        let lines = render(Source::Html(html), 40, &p);
        let title = span(&lines, "Title");
        assert_eq!(title.style.fg, Some(p.primary));
        assert!(title.style.add_modifier.contains(Modifier::BOLD));
        let link = span(&lines, "link");
        assert_eq!(link.style.fg, Some(p.info));
        assert!(link.style.add_modifier.contains(Modifier::UNDERLINED));
        let gone = span(&lines, "gone");
        assert!(gone.style.add_modifier.contains(Modifier::CROSSED_OUT));
        let shot = span(&lines, "shot");
        assert_eq!(shot.style.fg, Some(p.fg_mute));
        assert!(shot.style.add_modifier.contains(Modifier::ITALIC));
        assert_eq!(span(&lines, "said").style.fg, Some(p.fg_mute));
    }

    #[test]
    fn wrap_keeps_styles_across_breaks() {
        let p = Palette::pierre();
        let lines = render(Source::Markdown("one *two three four* five"), 10, &p);
        assert_eq!(texts(&lines), ["one two", "three four", "five"]);
        let italic = |l: &Line<'_>, word: &str| {
            l.spans
                .iter()
                .find(|s| s.content.contains(word))
                .is_some_and(|s| s.style.add_modifier.contains(Modifier::ITALIC))
        };
        assert!(italic(&lines[1], "three"));
        assert!(!italic(&lines[2], "five"));
    }
}
