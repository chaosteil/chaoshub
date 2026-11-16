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
        CellStyle::Normal | CellStyle::Markdown | CellStyle::Html | CellStyle::DiffFile => {
            palette.fg
        }
        CellStyle::Dim => palette.fg_mute,
        CellStyle::Primary => palette.primary,
        CellStyle::Success | CellStyle::DiffAdd => palette.success,
        CellStyle::Danger | CellStyle::DiffDel => palette.danger,
        CellStyle::Warn => palette.warn,
        CellStyle::Info => palette.info,
        CellStyle::DiffHunk => palette.diff_hunk,
        CellStyle::DiffGutter => palette.diff_gutter,
        // The merge color is too dark for text on the background, so merge
        // text uses the lighter purple.
        CellStyle::Merge | CellStyle::Badge(_) | CellStyle::MergeBadge(_) => palette.purple,
    };
    let style = Style::new().fg(fg);
    // Danger and warn carry a state, so they stand out from plain text.
    // The name of an open file is a header, so it is bold too. A removed
    // diff line is not a state, so it stays regular.
    if matches!(
        cell,
        CellStyle::Danger | CellStyle::Warn | CellStyle::DiffFile
    ) {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

/// The background of a whole row of `cells`: the tint of the first diff
/// cell, or `None` for a row that keeps the background of the pane.
pub(crate) fn row_bg(cells: &[Cell], palette: &Palette) -> Option<Color> {
    cells.iter().find_map(|c| match c.style {
        CellStyle::DiffAdd => Some(palette.diff_add_bg),
        CellStyle::DiffDel => Some(palette.diff_del_bg),
        CellStyle::DiffHunk => Some(palette.diff_hunk_bg),
        CellStyle::DiffFile => Some(palette.surface),
        _ => None,
    })
}

/// The color at `mix` between `from` (0) and `to` (255), mixed in RGB.
/// Colors that are not RGB, as in the 16-color palette, do not mix: the
/// color stays `from`, so the merge badge does not pulse. It switches to
/// the purple only when the pull request shows MERGED.
fn blend(from: Color, to: Color, mix: u8) -> Color {
    if let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (from, to) {
        let channel = |a: u8, b: u8| {
            let (a, b, mix) = (u32::from(a), u32::from(b), u32::from(mix));
            let value = (a * (255 - mix) + b * mix + 127) / 255;
            u8::try_from(value).unwrap_or(u8::MAX)
        };
        return Color::Rgb(channel(r1, r2), channel(g1, g2), channel(b1, b2));
    }
    from
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
    fn diff_cells_map_to_the_diff_colors() {
        let p = Palette::pierre();
        let fg = |style| cell_style(style, &p).fg;
        assert_eq!(fg(CellStyle::DiffAdd), Some(p.success));
        assert_eq!(fg(CellStyle::DiffDel), Some(p.danger));
        assert_eq!(fg(CellStyle::DiffHunk), Some(p.diff_hunk));
        assert_eq!(fg(CellStyle::DiffGutter), Some(p.diff_gutter));
        assert_eq!(fg(CellStyle::DiffFile), Some(p.fg));
        let del = cell_style(CellStyle::DiffDel, &p);
        assert!(!del.add_modifier.contains(Modifier::BOLD));
        assert_eq!(del.bg, None, "the row gives the tint");
    }

    #[test]
    fn the_first_diff_cell_gives_the_row_tint() {
        let p = Palette::pierre();
        let cell = |style| Cell {
            text: String::new(),
            style,
        };
        let bg = |styles: &[CellStyle]| {
            let cells: Vec<Cell> = styles.iter().map(|&s| cell(s)).collect();
            row_bg(&cells, &p)
        };
        assert_eq!(
            bg(&[CellStyle::DiffGutter, CellStyle::DiffAdd]),
            Some(p.diff_add_bg)
        );
        assert_eq!(bg(&[CellStyle::DiffDel]), Some(p.diff_del_bg));
        assert_eq!(bg(&[CellStyle::DiffHunk]), Some(p.diff_hunk_bg));
        assert_eq!(
            bg(&[CellStyle::DiffFile, CellStyle::Success]),
            Some(p.surface)
        );
        assert_eq!(bg(&[CellStyle::Normal, CellStyle::Success]), None);
        assert_eq!(bg(&[]), None);
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
    fn a_merge_badge_on_16_colors_stays_green_until_merged() {
        let p = Palette::ansi16();
        // Every mix that the pulse gives over the time of a merge.
        for mix in 0..=u8::MAX {
            let bg = cell_style(CellStyle::MergeBadge(mix), &p).bg;
            assert_eq!(bg, Some(Color::Green), "{mix}");
        }
        // The MERGED badge when the merge lands.
        let merged = cell_style(CellStyle::Badge(Tone::Merge), &p).bg;
        assert_eq!(merged, Some(Color::Magenta));
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
