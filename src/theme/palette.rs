//! The color palettes: Pierre for true-color terminals, and the 16 ANSI
//! colors for the others.

use ratatui::style::Color;

use super::{Palette, SyntaxColors};

impl Palette {
    /// The Pierre palette, from `pierre-theme.nvim`.
    pub(crate) const fn pierre() -> Self {
        Self {
            bg: Color::Rgb(0x0a, 0x0a, 0x0a),
            surface: Color::Rgb(0x17, 0x17, 0x17),
            fg: Color::Rgb(0xfa, 0xfa, 0xfa),
            fg_dim: Color::Rgb(0xd4, 0xd4, 0xd4),
            fg_mute: Color::Rgb(0xa3, 0xa3, 0xa3),
            primary: Color::Rgb(0x00, 0x9f, 0xff),
            success: Color::Rgb(0x07, 0xc4, 0x80),
            danger: Color::Rgb(0xff, 0x2e, 0x3f),
            warn: Color::Rgb(0xff, 0xca, 0x00),
            info: Color::Rgb(0x08, 0xc0, 0xef),
            merge: Color::Rgb(0x7b, 0x43, 0xf8),
            magenta: Color::Rgb(0xe1, 0x30, 0xac),
            orange: Color::Rgb(0xff, 0xa3, 0x59),
            purple: Color::Rgb(0x9d, 0x6a, 0xfb),
            pink: Color::Rgb(0xff, 0x67, 0x8d),
            // The diff tints are the success, danger and info colors at
            // 10% over the background, as in the diff colors of the Pierre
            // theme (`#07c4801a`, `#ff2e3f1a`).
            diff_add_bg: Color::Rgb(0x0a, 0x1d, 0x16),
            diff_del_bg: Color::Rgb(0x23, 0x0e, 0x0f),
            // A changed word has the success or the danger color at 20%
            // over the tint of its line, as the dark emphasis of the
            // Pierre diff view (`--diffs-bg-addition-emphasis`).
            diff_add_word_bg: Color::Rgb(0x09, 0x3e, 0x2b),
            diff_del_word_bg: Color::Rgb(0x4f, 0x14, 0x19),
            diff_hunk: Color::Rgb(0x7e, 0xa6, 0xc0),
            diff_hunk_bg: Color::Rgb(0x0a, 0x1d, 0x21),
            // The line number color of the Pierre theme.
            diff_gutter: Color::Rgb(0x73, 0x73, 0x73),
            // The token colors of the `pierre-dark` theme.
            syntax: SyntaxColors {
                comment: Color::Rgb(0x73, 0x73, 0x73),
                string: Color::Rgb(0x5e, 0xcc, 0x71),
                number: Color::Rgb(0x68, 0xcd, 0xf2),
                constant: Color::Rgb(0xff, 0xd4, 0x52),
                keyword: Color::Rgb(0xff, 0x67, 0x8d),
                operator: Color::Rgb(0x08, 0xc0, 0xef),
                function: Color::Rgb(0x9d, 0x6a, 0xfb),
                type_name: Color::Rgb(0xd5, 0x68, 0xea),
                namespace: Color::Rgb(0xff, 0xab, 0x16),
                variable: Color::Rgb(0xff, 0xa3, 0x59),
                parameter: Color::Rgb(0xa3, 0xa3, 0xa3),
                punctuation: Color::Rgb(0x63, 0x63, 0x63),
            },
        }
    }

    /// The 16 ANSI colors, for terminals without true color.
    pub(crate) const fn ansi16() -> Self {
        Self {
            bg: Color::Black,
            surface: Color::Black,
            fg: Color::White,
            fg_dim: Color::Gray,
            fg_mute: Color::DarkGray,
            primary: Color::LightBlue,
            success: Color::Green,
            danger: Color::Red,
            warn: Color::Yellow,
            info: Color::Cyan,
            merge: Color::Magenta,
            magenta: Color::LightMagenta,
            orange: Color::LightYellow,
            purple: Color::Magenta,
            pink: Color::LightRed,
            // 16 colors have no subtle tint, so the diff lines keep the
            // background.
            diff_add_bg: Color::Black,
            diff_del_bg: Color::Black,
            // A changed word is the only part of a changed line with a
            // background. The green or the red text of its line stays
            // readable on dark gray.
            diff_add_word_bg: Color::DarkGray,
            diff_del_word_bg: Color::DarkGray,
            diff_hunk: Color::Cyan,
            diff_hunk_bg: Color::Black,
            diff_gutter: Color::DarkGray,
            syntax: SyntaxColors {
                comment: Color::DarkGray,
                string: Color::Green,
                number: Color::Cyan,
                constant: Color::Yellow,
                keyword: Color::LightRed,
                operator: Color::Cyan,
                function: Color::Magenta,
                type_name: Color::LightMagenta,
                namespace: Color::Yellow,
                variable: Color::LightYellow,
                parameter: Color::Gray,
                punctuation: Color::DarkGray,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The relative luminance of an sRGB color (WCAG 2).
    fn luminance(color: Color) -> f64 {
        let Color::Rgb(r, g, b) = color else {
            panic!("not an rgb color: {color:?}");
        };
        let channel = |c: u8| {
            let c = f64::from(c) / 255.0;
            if c <= 0.039_28 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
    }

    /// The WCAG contrast ratio of two colors.
    fn contrast(a: Color, b: Color) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn pierre_has_the_theme_colors() {
        let p = Palette::pierre();
        assert_eq!(p.bg, Color::Rgb(0x0a, 0x0a, 0x0a));
        assert_eq!(p.primary, Color::Rgb(0x00, 0x9f, 0xff));
        assert_eq!(p.merge, Color::Rgb(0x7b, 0x43, 0xf8));
    }

    #[test]
    fn pierre_text_colors_keep_contrast() {
        let p = Palette::pierre();
        let text = [
            ("fg", p.fg),
            ("fg_dim", p.fg_dim),
            ("fg_mute", p.fg_mute),
            ("primary", p.primary),
            ("success", p.success),
            ("danger", p.danger),
            ("warn", p.warn),
            ("info", p.info),
            ("magenta", p.magenta),
            ("orange", p.orange),
            ("purple", p.purple),
            ("pink", p.pink),
        ];
        for (name, color) in text {
            let ratio = contrast(color, p.bg);
            assert!(ratio >= 4.5, "{name} on bg: {ratio:.2}");
        }
    }

    #[test]
    fn pierre_status_colors_keep_contrast_on_surface() {
        let p = Palette::pierre();
        let status = [
            ("fg", p.fg),
            ("fg_mute", p.fg_mute),
            ("primary", p.primary),
            ("success", p.success),
            ("danger", p.danger),
            ("warn", p.warn),
        ];
        for (name, color) in status {
            let ratio = contrast(color, p.surface);
            assert!(ratio >= 4.5, "{name} on surface: {ratio:.2}");
        }
    }

    #[test]
    fn pierre_diff_lines_keep_contrast() {
        let p = Palette::pierre();
        // The text color of each diff line on the tint of its row.
        let lines = [
            ("success on diff_add_bg", p.success, p.diff_add_bg),
            ("fg on diff_add_bg", p.fg, p.diff_add_bg),
            ("danger on diff_del_bg", p.danger, p.diff_del_bg),
            ("fg on diff_del_bg", p.fg, p.diff_del_bg),
            ("diff_hunk on diff_hunk_bg", p.diff_hunk, p.diff_hunk_bg),
        ];
        for (name, text, row) in lines {
            let ratio = contrast(text, row);
            assert!(ratio >= 4.5, "{name}: {ratio:.2}");
        }
        // The text of a changed word on the word tint. The danger color
        // is the text only in a file without syntax colors, and only on
        // some words, so 3:1 is enough for it.
        for (name, text, word) in [
            ("fg on diff_add_word_bg", p.fg, p.diff_add_word_bg),
            ("fg on diff_del_word_bg", p.fg, p.diff_del_word_bg),
            ("success on diff_add_word_bg", p.success, p.diff_add_word_bg),
        ] {
            let ratio = contrast(text, word);
            assert!(ratio >= 4.5, "{name}: {ratio:.2}");
        }
        let ratio = contrast(p.danger, p.diff_del_word_bg);
        assert!(ratio >= 3.0, "danger on diff_del_word_bg: {ratio:.2}");
        // The word tint stands out from the tint of its line.
        assert!(luminance(p.diff_add_word_bg) > luminance(p.diff_add_bg));
        assert!(luminance(p.diff_del_word_bg) > luminance(p.diff_del_bg));
        // The line numbers are a dim aid, as in the Pierre theme, so 3:1
        // is enough for them.
        for row in [p.bg, p.diff_add_bg, p.diff_del_bg] {
            let ratio = contrast(p.diff_gutter, row);
            assert!(ratio >= 3.0, "diff_gutter on {row:?}: {ratio:.2}");
        }
    }

    #[test]
    fn ansi16_diff_lines_keep_the_background() {
        let p = Palette::ansi16();
        assert_eq!(p.diff_add_bg, p.bg);
        assert_eq!(p.diff_del_bg, p.bg);
        assert_eq!(p.diff_hunk_bg, p.bg);
        assert_ne!(p.diff_add_word_bg, p.bg, "a changed word stands out");
        assert_ne!(p.diff_del_word_bg, p.bg, "a changed word stands out");
    }

    #[test]
    fn ansi16_uses_named_colors_only() {
        let p = Palette::ansi16();
        let all = [
            p.bg,
            p.surface,
            p.fg,
            p.fg_dim,
            p.fg_mute,
            p.primary,
            p.success,
            p.danger,
            p.warn,
            p.info,
            p.merge,
            p.magenta,
            p.orange,
            p.purple,
            p.pink,
            p.diff_add_bg,
            p.diff_del_bg,
            p.diff_add_word_bg,
            p.diff_del_word_bg,
            p.diff_hunk,
            p.diff_hunk_bg,
            p.diff_gutter,
            p.syntax.comment,
            p.syntax.string,
            p.syntax.number,
            p.syntax.constant,
            p.syntax.keyword,
            p.syntax.operator,
            p.syntax.function,
            p.syntax.type_name,
            p.syntax.namespace,
            p.syntax.variable,
            p.syntax.parameter,
            p.syntax.punctuation,
        ];
        assert!(
            all.iter()
                .all(|c| !matches!(c, Color::Rgb(..) | Color::Indexed(_)))
        );
    }

    #[test]
    fn pierre_bands_keep_contrast() {
        let p = Palette::pierre();
        // Black text on the selection band, the inverted headers, the
        // active tab and the status band.
        let bands = [
            ("primary", p.primary),
            ("info", p.info),
            ("warn", p.warn),
            ("danger", p.danger),
            ("success", p.success),
            ("fg_mute", p.fg_mute),
        ];
        for (name, color) in bands {
            let ratio = contrast(p.bg, color);
            assert!(ratio >= 4.5, "bg on {name}: {ratio:.2}");
        }
    }
}
