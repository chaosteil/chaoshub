//! Maps the semantic styles of the models to theme colors.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::app::models::{Cell, CellStyle, Tone};
use crate::theme::Palette;

/// The style of a cell. Only the foreground and the modifiers; the
/// background comes from the row or the bar. A badge also sets the
/// background: the tone color, or for a merge badge the mix of the
/// success color and the purple.
pub(crate) fn cell_style(cell: CellStyle, palette: &Palette) -> Style {
    if let CellStyle::MergeBadge(mix) = cell {
        return Style::new()
            .fg(palette.bg)
            .bg(blend(palette.success, palette.purple, mix))
            .add_modifier(Modifier::BOLD);
    }
    if let CellStyle::Badge(tone) = cell {
        let bg = match tone {
            Tone::Success => palette.success,
            Tone::Danger => palette.danger,
            Tone::Warn => palette.warn,
            Tone::Merge => palette.purple,
            Tone::Dim => palette.fg_mute,
        };
        return Style::new()
            .fg(palette.bg)
            .bg(bg)
            .add_modifier(Modifier::BOLD);
    }
    let fg = match cell {
        CellStyle::Normal | CellStyle::Markdown | CellStyle::Html => palette.fg,
        CellStyle::Dim => palette.fg_mute,
        CellStyle::Primary => palette.primary,
        CellStyle::Success => palette.success,
        CellStyle::Danger => palette.danger,
        CellStyle::Warn => palette.warn,
        CellStyle::Info => palette.info,
        // The merge color is too dark for text on the background, so merge
        // text uses the lighter purple.
        CellStyle::Merge | CellStyle::Badge(_) | CellStyle::MergeBadge(_) => palette.purple,
    };
    let style = Style::new().fg(fg);
    // Danger and warn carry a state, so they stand out from plain text.
    if matches!(cell, CellStyle::Danger | CellStyle::Warn) {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

/// The color at `mix` between `from` (0) and `to` (255), mixed in RGB.
/// Colors that are not RGB, as in the 16-color palette, do not mix: the
/// color switches from `from` to `to` at the half.
fn blend(from: Color, to: Color, mix: u8) -> Color {
    if let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (from, to) {
        let channel = |a: u8, b: u8| {
            let (a, b, mix) = (u32::from(a), u32::from(b), u32::from(mix));
            let value = (a * (255 - mix) + b * mix + 127) / 255;
            u8::try_from(value).unwrap_or(u8::MAX)
        };
        return Color::Rgb(channel(r1, r2), channel(g1, g2), channel(b1, b2));
    }
    if mix < 128 { from } else { to }
}

/// A line of cells, each in its style.
pub(crate) fn line(cells: &[Cell], palette: &Palette) -> Line<'static> {
    Line::from(
        cells
            .iter()
            .map(|c| Span::styled(c.text.clone(), cell_style(c.style, palette)))
            .collect::<Vec<_>>(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_map_to_palette_colors() {
        let p = Palette::pierre();
        assert_eq!(cell_style(CellStyle::Normal, &p).fg, Some(p.fg));
        assert_eq!(cell_style(CellStyle::Dim, &p).fg, Some(p.fg_mute));
        assert_eq!(cell_style(CellStyle::Merge, &p).fg, Some(p.purple));
        assert!(
            cell_style(CellStyle::Danger, &p)
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn a_merge_badge_mixes_the_success_color_and_the_purple() {
        let p = Palette::pierre();
        let bg = |mix| cell_style(CellStyle::MergeBadge(mix), &p).bg;
        assert_eq!(bg(0), Some(p.success));
        assert_eq!(bg(255), Some(p.purple));
        // The half of #07c480 and #9d6afb.
        assert_eq!(bg(128), Some(Color::Rgb(0x52, 0x97, 0xbe)));
        let style = cell_style(CellStyle::MergeBadge(64), &p);
        assert_eq!(style.fg, Some(p.bg));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn a_merge_badge_on_16_colors_switches_without_a_fade() {
        let p = Palette::ansi16();
        let bg = |mix| cell_style(CellStyle::MergeBadge(mix), &p).bg;
        for mix in [0, 64, 127] {
            assert_eq!(bg(mix), Some(Color::Green), "{mix}");
        }
        for mix in [128, 192, 255] {
            assert_eq!(bg(mix), Some(Color::Magenta), "{mix}");
        }
    }

    #[test]
    fn badges_are_bold_bg_on_the_tone_color() {
        let p = Palette::pierre();
        let tones = [
            (Tone::Success, p.success),
            (Tone::Danger, p.danger),
            (Tone::Warn, p.warn),
            (Tone::Merge, p.purple),
            (Tone::Dim, p.fg_mute),
        ];
        for (tone, color) in tones {
            let style = cell_style(CellStyle::Badge(tone), &p);
            assert_eq!(style.fg, Some(p.bg), "{tone:?}");
            assert_eq!(style.bg, Some(color), "{tone:?}");
            assert!(style.add_modifier.contains(Modifier::BOLD), "{tone:?}");
        }
    }
}
