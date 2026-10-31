//! Text helpers: display width, truncation and alignment.

use textwrap::core::display_width;

use crate::app::models::Align;

/// The display width of a text in cells.
pub(crate) fn width(text: &str) -> usize {
    display_width(text)
}

/// The display width of a char in cells.
fn char_width(c: char) -> usize {
    let mut bytes = [0; 4];
    display_width(c.encode_utf8(&mut bytes))
}

/// Cuts a text to `max` cells. A cut text ends with `…`.
pub(crate) fn truncate(text: &str, max: usize) -> String {
    if text.is_ascii() && text.len() <= max {
        return text.to_owned();
    }
    if width(text) <= max {
        return text.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = char_width(c);
        if used + w > max - 1 {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

/// Cuts a text to `max` cells and pads it to exactly `max` cells with the
/// alignment.
pub(crate) fn fit(text: &str, max: usize, align: Align) -> String {
    let text = truncate(text, max);
    let pad = max.saturating_sub(width(&text));
    match align {
        Align::Left => format!("{text}{}", " ".repeat(pad)),
        Align::Right => format!("{}{text}", " ".repeat(pad)),
        Align::Center => {
            let left = pad / 2;
            format!("{}{text}{}", " ".repeat(left), " ".repeat(pad - left))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_adds_an_ellipsis() {
        assert_eq!(truncate("hello world", 8), "hello w…");
        assert_eq!(truncate("hello", 8), "hello");
        assert_eq!(truncate("hello", 0), "");
    }

    #[test]
    fn truncate_counts_wide_chars() {
        assert_eq!(truncate("日本語の文", 5), "日本…");
    }

    #[test]
    fn fit_aligns() {
        assert_eq!(fit("ab", 5, Align::Left), "ab   ");
        assert_eq!(fit("ab", 5, Align::Right), "   ab");
        assert_eq!(fit("ab", 5, Align::Center), " ab  ");
    }
}
