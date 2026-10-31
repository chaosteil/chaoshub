//! The color palettes: Pierre for true-color terminals and the 16 ANSI
//! colors for the others.

use ratatui::style::Color;

use super::Palette;

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
    fn ansi16_uses_named_colors_only() {
        let p = Palette::ansi16();
        let all = [
            p.bg, p.surface, p.fg, p.fg_dim, p.fg_mute, p.primary, p.success, p.danger, p.warn,
            p.info, p.merge, p.magenta, p.orange, p.purple, p.pink,
        ];
        assert!(
            all.iter()
                .all(|c| !matches!(c, Color::Rgb(..) | Color::Indexed(_)))
        );
    }
}
