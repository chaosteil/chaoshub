//! The colors of the 16 ANSI numbers, the nearest ANSI color of a hex
//! color, and the text color that stays readable on a background.

use ratatui::style::Color;

/// The 16 ANSI colors in the order of their numbers, with the default RGB
/// value of xterm. The RGB value is only an estimate, because each
/// terminal can set its own colors.
const ANSI: [(Color, (u8, u8, u8)); 16] = [
    (Color::Black, (0x00, 0x00, 0x00)),
    (Color::Red, (0xcd, 0x00, 0x00)),
    (Color::Green, (0x00, 0xcd, 0x00)),
    (Color::Yellow, (0xcd, 0xcd, 0x00)),
    (Color::Blue, (0x00, 0x00, 0xee)),
    (Color::Magenta, (0xcd, 0x00, 0xcd)),
    (Color::Cyan, (0x00, 0xcd, 0xcd)),
    (Color::Gray, (0xe5, 0xe5, 0xe5)),
    (Color::DarkGray, (0x7f, 0x7f, 0x7f)),
    (Color::LightRed, (0xff, 0x00, 0x00)),
    (Color::LightGreen, (0x00, 0xff, 0x00)),
    (Color::LightYellow, (0xff, 0xff, 0x00)),
    (Color::LightBlue, (0x5c, 0x5c, 0xff)),
    (Color::LightMagenta, (0xff, 0x00, 0xff)),
    (Color::LightCyan, (0x00, 0xff, 0xff)),
    (Color::White, (0xff, 0xff, 0xff)),
];

/// The ANSI color of the number `index`, 0 to 15, as in the
/// configuration (`blue` is 4, `bright_magenta` is 13). `None` for a
/// higher number.
pub(crate) fn ansi(index: u8) -> Option<Color> {
    ANSI.get(usize::from(index)).map(|&(color, _)| color)
}

/// The RGB value of a color. An ANSI color gets the estimate of the
/// table. `None` for the other colors, as `Reset` or an indexed color.
fn rgb(color: Color) -> Option<(u8, u8, u8)> {
    if let Color::Rgb(r, g, b) = color {
        return Some((r, g, b));
    }
    ANSI.iter()
        .find(|(ansi, _)| *ansi == color)
        .map(|&(_, rgb)| rgb)
}

/// The ANSI color nearest to `color`, by the distance of the RGB values.
/// Every other color stays the same.
pub(crate) fn nearest_ansi(color: Color) -> Color {
    let Color::Rgb(r, g, b) = color else {
        return color;
    };
    let distance = |(ar, ag, ab): (u8, u8, u8)| {
        let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2);
        d(ar, r) + d(ag, g) + d(ab, b)
    };
    ANSI.iter()
        .min_by_key(|&&(_, rgb)| distance(rgb))
        .map_or(color, |&(ansi, _)| ansi)
}

/// The relative luminance of a color (WCAG 2). `None` when the color has
/// no RGB value.
fn luminance(color: Color) -> Option<f64> {
    let (r, g, b) = rgb(color)?;
    let channel = |c: u8| {
        let c = f64::from(c) / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    Some(0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b))
}

/// The WCAG contrast ratio of two colors.
fn contrast(a: f64, b: f64) -> f64 {
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// The text color for the background `bg`: `dark` or `light`, the one
/// with more contrast. `dark` wins a tie, and also when a color has no
/// RGB value.
pub(crate) fn readable_on(bg: Color, dark: Color, light: Color) -> Color {
    match (luminance(bg), luminance(dark), luminance(light)) {
        (Some(bg), Some(d), Some(l)) if contrast(bg, l) > contrast(bg, d) => light,
        _ => dark,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_numbers_map_to_the_colors() {
        assert_eq!(ansi(0), Some(Color::Black));
        assert_eq!(ansi(4), Some(Color::Blue));
        assert_eq!(ansi(7), Some(Color::Gray));
        assert_eq!(ansi(8), Some(Color::DarkGray));
        assert_eq!(ansi(13), Some(Color::LightMagenta));
        assert_eq!(ansi(15), Some(Color::White));
        assert_eq!(ansi(16), None);
    }

    #[test]
    fn nearest_ansi_picks_the_closest_color() {
        assert_eq!(
            nearest_ansi(Color::Rgb(0xff, 0xff, 0x00)),
            Color::LightYellow
        );
        assert_eq!(nearest_ansi(Color::Rgb(0xd0, 0xc8, 0x10)), Color::Yellow);
        assert_eq!(nearest_ansi(Color::Rgb(0x10, 0x10, 0x10)), Color::Black);
        assert_eq!(nearest_ansi(Color::Rgb(0xff, 0x5f, 0x87)), Color::Magenta);
        assert_eq!(nearest_ansi(Color::Rgb(0x80, 0x80, 0x80)), Color::DarkGray);
        // A named color stays the same.
        assert_eq!(nearest_ansi(Color::Blue), Color::Blue);
        // The estimate of each ANSI color maps back to it.
        for (color, (r, g, b)) in ANSI {
            assert_eq!(nearest_ansi(Color::Rgb(r, g, b)), color);
        }
    }

    #[test]
    fn readable_on_picks_black_or_white() {
        let (dark, light) = (Color::Rgb(0x0a, 0x0a, 0x0a), Color::Rgb(0xfa, 0xfa, 0xfa));
        // The blue of Pierre, yellow and pink get black text.
        for bg in [
            Color::Rgb(0x00, 0x9f, 0xff),
            Color::Rgb(0xff, 0xff, 0x00),
            Color::Rgb(0xff, 0x5f, 0x87),
            Color::Yellow,
            Color::LightYellow,
        ] {
            assert_eq!(readable_on(bg, dark, light), dark, "{bg:?}");
        }
        // Dark colors get white text.
        for bg in [
            Color::Rgb(0x5f, 0x00, 0xaf),
            Color::Rgb(0x00, 0x00, 0x80),
            Color::Blue,
            Color::Red,
            Color::Black,
        ] {
            assert_eq!(readable_on(bg, dark, light), light, "{bg:?}");
        }
        // The named colors of the ANSI palette work too.
        assert_eq!(
            readable_on(Color::Blue, Color::Black, Color::White),
            Color::White
        );
        assert_eq!(
            readable_on(Color::Cyan, Color::Black, Color::White),
            Color::Black
        );
        // Without an RGB value, the dark color wins.
        assert_eq!(readable_on(Color::Reset, dark, light), dark);
    }
}
