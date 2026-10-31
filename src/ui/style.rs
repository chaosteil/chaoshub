//! Maps the semantic styles of the models to theme colors.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::app::models::{Cell, CellStyle};
use crate::theme::Palette;

/// The style of a cell. Only the foreground and the modifiers; the
/// background comes from the row or the bar.
pub(crate) fn cell_style(cell: CellStyle, palette: &Palette) -> Style {
    let fg = match cell {
        CellStyle::Normal => palette.fg,
        CellStyle::Dim => palette.fg_mute,
        CellStyle::Primary => palette.primary,
        CellStyle::Success => palette.success,
        CellStyle::Danger => palette.danger,
        CellStyle::Warn => palette.warn,
        CellStyle::Info => palette.info,
        // The merge color is too dark for text on the background, so merge
        // text uses the lighter purple.
        CellStyle::Merge => palette.purple,
        CellStyle::Local => palette.orange,
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
        assert_eq!(cell_style(CellStyle::Local, &p).fg, Some(p.orange));
        assert_eq!(cell_style(CellStyle::Merge, &p).fg, Some(p.purple));
        assert!(
            cell_style(CellStyle::Danger, &p)
                .add_modifier
                .contains(Modifier::BOLD)
        );
    }
}
