//! Renders comment bodies as markdown, with a plain text fallback.
//!
//! tui-markdown turns the body into styled lines: headings, emphasis,
//! links, lists, quotes, and code spans and blocks in the code colors.
//! The lines are then word-wrapped to the width. When tui-markdown gives
//! no lines for a body with text, textwrap wraps the plain text instead.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use tui_markdown::{Options, StyleSheet};

use crate::theme::Palette;
use crate::ui::text;

/// The markdown styles in the theme colors.
#[derive(Debug, Clone, Copy)]
struct Sheet {
    palette: Palette,
}

impl StyleSheet for Sheet {
    fn heading(&self, level: u8) -> Style {
        let style = Style::new()
            .fg(self.palette.primary)
            .add_modifier(Modifier::BOLD);
        if level > 2 {
            style.add_modifier(Modifier::ITALIC)
        } else {
            style
        }
    }

    fn code(&self) -> Style {
        Style::new()
            .fg(self.palette.orange)
            .bg(self.palette.surface)
    }

    fn link(&self) -> Style {
        Style::new()
            .fg(self.palette.info)
            .add_modifier(Modifier::UNDERLINED)
    }

    fn blockquote(&self) -> Style {
        Style::new().fg(self.palette.fg_mute)
    }

    fn heading_meta(&self) -> Style {
        Style::new().fg(self.palette.fg_mute)
    }

    fn metadata_block(&self) -> Style {
        Style::new().fg(self.palette.fg_mute)
    }

    fn html(&self) -> Style {
        Style::new().fg(self.palette.fg_mute)
    }

    fn list_marker(&self) -> Style {
        Style::new().fg(self.palette.primary)
    }

    fn table_header(&self) -> Style {
        Style::new()
            .fg(self.palette.fg)
            .add_modifier(Modifier::BOLD)
    }

    fn table_border(&self) -> Style {
        Style::new().fg(self.palette.fg_mute)
    }
}

/// Renders a body to lines of at most `width` cells.
pub(crate) fn render(body: &str, width: u16, palette: &Palette) -> Vec<Line<'static>> {
    let width = usize::from(width.max(1));
    let options = Options::new(Sheet { palette: *palette });
    let parsed = tui_markdown::from_str_with_options(body, &options);
    let lines: Vec<Line<'static>> = parsed
        .lines
        .into_iter()
        .map(owned)
        .flat_map(|line| wrap(line, width))
        .collect();
    let has_text = |lines: &[Line<'_>]| lines.iter().any(|l| l.width() > 0);
    if !has_text(&lines) && !body.trim().is_empty() {
        return plain(body, width, palette);
    }
    lines
}

/// The plain text of a body, wrapped with textwrap.
pub(crate) fn plain(body: &str, width: usize, palette: &Palette) -> Vec<Line<'static>> {
    let style = Style::new().fg(palette.fg);
    body.lines()
        .flat_map(|line| {
            if line.is_empty() {
                vec![Line::default()]
            } else {
                textwrap::wrap(line, width)
                    .into_iter()
                    .map(|part| Line::styled(part.into_owned(), style))
                    .collect()
            }
        })
        .collect()
}

/// A line that owns its text.
fn owned(line: Line<'_>) -> Line<'static> {
    Line {
        style: line.style,
        alignment: line.alignment,
        spans: line
            .spans
            .into_iter()
            .map(|s| Span::styled(s.content.into_owned(), s.style))
            .collect(),
    }
}

/// A piece of a span: a word or a run of spaces.
struct Piece {
    text: String,
    style: Style,
    space: bool,
}

/// Splits spans into words and runs of spaces, each with its style.
fn pieces(line: &Line<'static>) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    for span in &line.spans {
        for c in span.content.chars() {
            let space = c == ' ';
            match out.last_mut() {
                Some(last) if last.space == space && last.style == span.style => last.text.push(c),
                _ => out.push(Piece {
                    text: c.to_string(),
                    style: span.style,
                    space,
                }),
            }
        }
    }
    out
}

/// Word-wraps a styled line to `width` cells. A word longer than the
/// width is cut into parts. Spaces at a break are dropped.
fn wrap(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    if line.width() <= width {
        return vec![line];
    }
    let mut lines = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    let flush = |spans: &mut Vec<Span<'static>>, lines: &mut Vec<Line<'static>>| {
        while spans.last().is_some_and(|s| s.content.trim().is_empty()) {
            spans.pop();
        }
        lines.push(Line::from(std::mem::take(spans)).style(line.style));
    };
    for piece in pieces(&line) {
        let w = text::width(&piece.text);
        if piece.space {
            if used + w <= width && used > 0 {
                spans.push(Span::styled(piece.text, piece.style));
                used += w;
            } else if used > 0 {
                flush(&mut spans, &mut lines);
                used = 0;
            }
            continue;
        }
        if used + w > width && used > 0 {
            flush(&mut spans, &mut lines);
            used = 0;
        }
        let mut rest = piece.text.as_str();
        while text::width(rest) > width - used {
            let cut = split_at_width(rest, width - used);
            if cut == 0 {
                flush(&mut spans, &mut lines);
                used = 0;
                continue;
            }
            spans.push(Span::styled(rest[..cut].to_owned(), piece.style));
            flush(&mut spans, &mut lines);
            used = 0;
            rest = &rest[cut..];
        }
        if !rest.is_empty() {
            used += text::width(rest);
            spans.push(Span::styled(rest.to_owned(), piece.style));
        }
    }
    if !spans.is_empty() {
        flush(&mut spans, &mut lines);
    }
    lines
}

/// The byte index after the chars of `s` that fit in `max` cells.
fn split_at_width(s: &str, max: usize) -> usize {
    let mut used = 0;
    for (i, c) in s.char_indices() {
        let mut bytes = [0; 4];
        let w = text::width(c.encode_utf8(&mut bytes));
        if used + w > max {
            return i;
        }
        used += w;
    }
    s.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn markdown_body() {
        let body = "## Review\n\nThe `Transitions::apply` call must run **before** `prepare`.\n\n\
                    - one\n- two\n\n> quoted text\n\n```rust\nlet x = 1;\n```";
        let lines = render(body, 40, &Palette::pierre());
        insta::assert_snapshot!(texts(&lines).join("\n"));
        assert!(lines.iter().all(|l| l.width() <= 40));
    }

    #[test]
    fn code_spans_use_the_code_colors() {
        let p = Palette::pierre();
        let lines = render("call `apply` first", 40, &p);
        let code = lines[0]
            .spans
            .iter()
            .find(|s| s.content.contains("apply"))
            .unwrap();
        assert_eq!(code.style.fg, Some(p.orange));
        assert_eq!(code.style.bg, Some(p.surface));
    }

    #[test]
    fn long_words_are_cut() {
        let lines = render("aaaaaaaaaaaaaaaaaaaaaaaaa", 10, &Palette::pierre());
        assert_eq!(texts(&lines), ["aaaaaaaaaa", "aaaaaaaaaa", "aaaaa"]);
    }

    #[test]
    fn wrap_keeps_styles_across_breaks() {
        let p = Palette::pierre();
        let lines = render("one *two three four* five", 10, &p);
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

    #[test]
    fn plain_fallback_wraps_with_textwrap() {
        let lines = plain("one two three four five", 10, &Palette::pierre());
        assert_eq!(texts(&lines), ["one two", "three four", "five"]);
    }
}
