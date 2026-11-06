//! Maps the semantic styles of the models to theme colors.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::app::models::{Cell, CellStyle, Tone};
use crate::theme::Palette;

/// The style of a cell. Only the foreground and the modifiers; the
/// background comes from the row or the bar. A badge also sets the
/// background: the tone color.
pub(crate) fn cell_style(cell: CellStyle, palette: &Palette) -> Style {
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
        CellStyle::Merge | CellStyle::Badge(_) => palette.purple,
    };
    let style = Style::new().fg(fg);
    // Danger and warn carry a state, so they stand out from plain text.
    if matches!(cell, CellStyle::Danger | CellStyle::Warn) {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
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
