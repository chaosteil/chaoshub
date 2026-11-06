//! Renders comment bodies to rows of styled text.
//!
//! A body is HTML that GitHub rendered, or local markdown. pulldown-cmark
//! turns markdown into HTML. html2text then wraps the HTML to the width:
//! paragraphs, lists, quotes, code blocks, details and tables. Each piece
//! of a row has marks, for example `strong` or `code`. The ui turns the
//! marks into theme colors.
//!
//! The rows of each source and width stay in a cache, because the ui
//! renders the bodies again on each frame.
#![cfg_attr(
    not(test),
    expect(dead_code, reason = "the panes use the module in the next commit")
)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use html2text::render::RichAnnotation;
use pulldown_cmark::{Options, Parser};

/// The number of cached sources and widths. More entries clear the cache.
const CACHE_LIMIT: usize = 512;

/// The text of a body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source<'a> {
    /// Local markdown, for example the result of an agent.
    Markdown(&'a str),
    /// HTML that GitHub rendered.
    Html(&'a str),
}

/// The styles of a piece of text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Marks {
    pub strong: bool,
    pub emphasis: bool,
    pub code: bool,
    pub pre: bool,
    pub link: bool,
    /// The alt text of an image.
    pub image: bool,
    pub strike: bool,
    pub heading: bool,
    pub quote: bool,
    /// A box-drawing character of a table.
    pub border: bool,
}

/// A run of text with one set of marks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Piece {
    pub text: String,
    pub marks: Marks,
}

/// One row of a body on the screen.
pub(crate) type Row = Vec<Piece>;

/// The cached rows, by the hash of the source and the width.
type Cache = HashMap<(u64, usize), Rc<Vec<Row>>>;

thread_local! {
    /// The rows of each source and width.
    static CACHE: RefCell<Cache> = RefCell::new(HashMap::new());
}

/// The rows of `source`, wrapped to `width` cells. Only rows of
/// preformatted text can be wider.
pub(crate) fn rows(source: Source<'_>, width: usize) -> Rc<Vec<Row>> {
    let key = (hash(source), width);
    if let Some(rows) = CACHE.with(|cache| cache.borrow().get(&key).cloned()) {
        return rows;
    }
    let rows = Rc::new(render(source, width));
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() > CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(key, Rc::clone(&rows));
    });
    rows
}

/// The hash of the kind and the text of `source`.
fn hash(source: Source<'_>) -> u64 {
    let mut hasher = DefaultHasher::new();
    match source {
        Source::Markdown(text) => (0u8, text).hash(&mut hasher),
        Source::Html(text) => (1u8, text).hash(&mut hasher),
    }
    hasher.finish()
}

/// The rows of `source` at `width`, without the cache.
fn render(source: Source<'_>, width: usize) -> Vec<Row> {
    let (html, text) = match source {
        Source::Markdown(text) => (to_html(text), text),
        Source::Html(text) => (text.to_owned(), text),
    };
    let html = prepare(&html);
    match html2text::config::rich()
        .link_footnotes(false)
        .unicode_strikeout(false)
        .min_wrap_width(6)
        .allow_width_overflow()
        .lines_from_read(html.as_bytes(), width.max(1))
    {
        Ok(lines) => lines
            .iter()
            .map(|line| {
                row(line
                    .tagged_strings()
                    .map(|tagged| (tagged.s.as_str(), tagged.tag.as_slice())))
            })
            .collect(),
        Err(_) => plain(text, width),
    }
}

/// The HTML of a markdown text. Raw HTML in the markdown passes through.
fn to_html(markdown: &str) -> String {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_GFM;
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, Parser::new_ext(markdown, options));
    html
}

/// Changes the tags that html2text drops or shows plain:
///
/// - A checkbox `<input>` becomes `☑ ` when it is checked, else `☐ `.
/// - The text of a `<summary>` gets a `▸ ` and shows strong.
///
/// HTML comments stay as they are.
fn prepare(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        rest = &rest[start..];
        if rest.starts_with("<!--") {
            let end = rest.find("-->").map_or(rest.len(), |i| i + 3);
            out.push_str(&rest[..end]);
            rest = &rest[end..];
            continue;
        }
        let Some(end) = tag_end(rest) else {
            break;
        };
        let tag = &rest[..=end];
        rest = &rest[end + 1..];
        let inner = tag[1..tag.len() - 1].trim_end_matches('/');
        let name_len = inner
            .find(|c: char| c.is_whitespace())
            .unwrap_or(inner.len());
        let name = inner[..name_len].to_ascii_lowercase();
        let attrs = attributes(&inner[name_len..]);
        match name.as_str() {
            "input"
                if attrs
                    .iter()
                    .any(|(k, v)| k == "type" && v.eq_ignore_ascii_case("checkbox")) =>
            {
                if attrs.iter().any(|(k, _)| k == "checked") {
                    out.push_str("☑ ");
                } else {
                    out.push_str("☐ ");
                }
            }
            "summary" => out.push_str("<summary><strong>▸ "),
            "/summary" => out.push_str("</strong></summary>"),
            _ => out.push_str(tag),
        }
    }
    out.push_str(rest);
    out
}

/// The byte index of the `>` that ends the tag at the start of `s`. A `>`
/// in a quoted value does not end the tag.
fn tag_end(s: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

/// The attributes of a tag, as lowercase names and raw values. A name
/// without a value has an empty value.
fn attributes(s: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let mut chars = s.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace() || *c == '/').is_some() {}
        let mut name = String::new();
        while let Some(c) = chars.next_if(|c| !c.is_whitespace() && *c != '=' && *c != '/') {
            name.push(c.to_ascii_lowercase());
        }
        if name.is_empty() {
            return attrs;
        }
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let mut value = String::new();
        if chars.next_if_eq(&'=').is_some() {
            while chars.next_if(|c| c.is_whitespace()).is_some() {}
            match chars.next_if(|c| *c == '"' || *c == '\'') {
                Some(q) => {
                    for c in chars.by_ref() {
                        if c == q {
                            break;
                        }
                        value.push(c);
                    }
                }
                None => {
                    while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
                        value.push(c);
                    }
                }
            }
        }
        attrs.push((name, value));
    }
}

/// The marks of the annotations of a string.
fn marks(annotations: &[RichAnnotation]) -> Marks {
    let mut marks = Marks::default();
    for annotation in annotations {
        match annotation {
            RichAnnotation::Strong => marks.strong = true,
            RichAnnotation::Emphasis => marks.emphasis = true,
            RichAnnotation::Code => marks.code = true,
            RichAnnotation::Preformat(_) => marks.pre = true,
            RichAnnotation::Link(_) => marks.link = true,
            RichAnnotation::Image(_) => marks.image = true,
            RichAnnotation::Strikeout => marks.strike = true,
            _ => {}
        }
    }
    marks
}

/// True for a box-drawing character (U+2500 to U+257F).
fn is_border(c: char) -> bool {
    ('\u{2500}'..='\u{257f}').contains(&c)
}

/// True when `text` starts with 1 to 6 `#` and a space.
fn is_heading(text: &str) -> bool {
    let hashes = text.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&hashes) && text[hashes..].starts_with(' ')
}

/// The row of the tagged strings of one html2text line.
fn row<'a>(strings: impl Iterator<Item = (&'a str, &'a [RichAnnotation])>) -> Row {
    let strings: Vec<_> = strings.collect();
    let text: String = strings.iter().map(|(s, _)| *s).collect();
    let heading = is_heading(&text)
        && strings
            .first()
            .is_some_and(|(_, annotations)| annotations.is_empty());
    let quote = text.starts_with("> ");
    let mut row = Row::new();
    for (s, annotations) in strings {
        let mut base = marks(annotations);
        base.heading |= heading;
        base.quote |= quote;
        let mut rest = s;
        while let Some(c) = rest.chars().next() {
            let border = is_border(c);
            let len = rest
                .find(|c: char| is_border(c) != border)
                .unwrap_or(rest.len());
            row.push(Piece {
                text: rest[..len].to_owned(),
                marks: Marks { border, ..base },
            });
            rest = &rest[len..];
        }
    }
    row
}

/// The text wrapped with textwrap, with no marks.
fn plain(text: &str, width: usize) -> Vec<Row> {
    text.lines()
        .flat_map(|line| {
            if line.is_empty() {
                vec![Row::new()]
            } else {
                textwrap::wrap(line, width.max(1))
                    .into_iter()
                    .map(|part| {
                        vec![Piece {
                            text: part.into_owned(),
                            marks: Marks::default(),
                        }]
                    })
                    .collect()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HTML: &str = include_str!("testdata/body.html");
    const MARKDOWN: &str = include_str!("testdata/body.md");

    fn text(row: &Row) -> String {
        row.iter().map(|piece| piece.text.as_str()).collect()
    }

    fn texts(rows: &[Row]) -> Vec<String> {
        rows.iter().map(text).collect()
    }

    fn pieces<'a>(rows: &'a [Row], needle: &str) -> impl Iterator<Item = &'a Piece> {
        rows.iter()
            .flatten()
            .filter(move |piece| piece.text.contains(needle))
    }

    #[test]
    fn github_body() {
        let rows = rows(Source::Html(HTML), 36);
        insta::assert_snapshot!(texts(&rows).join("\n"));
    }

    #[test]
    fn rows_fit_the_width() {
        for width in [20, 36, 60] {
            for source in [Source::Html(HTML), Source::Markdown(MARKDOWN)] {
                for row in rows(source, width).iter() {
                    if row.iter().any(|piece| piece.marks.pre) {
                        continue;
                    }
                    let text = text(row);
                    assert!(
                        textwrap::core::display_width(&text) <= width,
                        "{text:?} is wider than {width}"
                    );
                }
            }
        }
    }

    #[test]
    fn comments_and_svg_do_not_show() {
        let all = texts(&rows(Source::Html(HTML), 60)).join("\n");
        assert!(!all.contains("Please fill"), "{all}");
        assert!(!all.contains("M0 6.75"), "{all}");
        assert!(!all.contains('<'), "{all}");
    }

    #[test]
    fn summary_checkboxes_and_images() {
        let rows = rows(Source::Html(HTML), 60);
        let summary: Vec<_> = pieces(&rows, "Click to see logs").collect();
        assert_ne!(summary.len(), 0);
        assert!(summary.iter().all(|piece| piece.marks.strong));
        let all = texts(&rows).join("\n");
        assert!(all.contains("▸ Click to see logs"), "{all}");
        assert!(all.contains("☑ done"), "{all}");
        assert!(all.contains("☐ todo"), "{all}");
        let image: Vec<_> = pieces(&rows, "screenshot of the bug").collect();
        assert_ne!(image.len(), 0, "{all}");
        assert!(image.iter().all(|piece| piece.marks.image));
    }

    #[test]
    fn marks_follow_the_tags() {
        let rows = rows(Source::Html(HTML), 60);
        let first = |needle| pieces(&rows, needle).next().unwrap().marks;
        assert!(first("Why").heading);
        assert!(first("bold").strong);
        assert!(first("code").code);
        assert!(first("@chaosteil").link);
        assert!(first("struck").strike);
        assert!(first("let").pre);
        assert!(first("─").border);
        assert!(!first("alpha").border);
    }

    #[test]
    fn tables_reflow_with_the_width() {
        let narrow = texts(&rows(Source::Html(HTML), 36));
        let wide = texts(&rows(Source::Html(HTML), 60));
        let table = |rows: &[String]| -> Vec<String> {
            rows.iter()
                .filter(|row| row.contains('│') || row.contains('─'))
                .cloned()
                .collect()
        };
        assert_ne!(table(&narrow), table(&wide));
        for rows in [&narrow, &wide] {
            assert!(rows.iter().any(|row| row.contains("Name")), "{rows:#?}");
        }
    }

    #[test]
    fn markdown_gives_the_same_text() {
        for width in [36, 60] {
            assert_eq!(
                texts(&rows(Source::Markdown(MARKDOWN), width)),
                texts(&rows(Source::Html(HTML), width)),
            );
        }
    }

    #[test]
    fn quotes_are_marked() {
        let rows = rows(Source::Markdown("> quoted text"), 40);
        assert!(pieces(&rows, "quoted").all(|piece| piece.marks.quote));
    }

    #[test]
    fn prepare_changes_checkboxes_and_summaries() {
        assert_eq!(
            prepare(
                r#"<input class="x checked" type="checkbox"> a <INPUT type=checkbox checked/> b"#
            ),
            "☐  a ☑  b"
        );
        assert_eq!(
            prepare(r#"<summary class="s">Hi</summary><!-- <summary> -->"#),
            "<summary><strong>▸ Hi</strong></summary><!-- <summary> -->"
        );
        assert_eq!(prepare(r#"<input type="text">"#), r#"<input type="text">"#);
    }

    #[test]
    fn the_cache_returns_the_same_rows() {
        let a = rows(Source::Markdown("cached *text*"), 30);
        let b = rows(Source::Markdown("cached *text*"), 30);
        assert!(Rc::ptr_eq(&a, &b));
        let html = rows(Source::Html("cached *text*"), 30);
        assert!(!Rc::ptr_eq(&a, &html));
    }
}
