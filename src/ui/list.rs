//! Renders a list model. Any tab produces one, so this renderer knows no
//! tab by name.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::app::models::{Column, ColumnWidth, ListModel};
use crate::theme::Theme;
use crate::ui::text;

/// The space between two columns.
pub(crate) const GAP: u16 = 1;

/// The width of each column for a total width. Fixed columns get their
/// width. Fill columns share the rest, each at least its minimum.
pub(crate) fn column_widths(columns: &[Column], width: u16) -> Vec<u16> {
    let gaps = GAP * u16::try_from(columns.len().saturating_sub(1)).unwrap_or(u16::MAX);
    let fixed: u16 = columns
        .iter()
        .map(|c| match c.width {
            ColumnWidth::Fixed(w) => w,
            ColumnWidth::Fill { .. } => 0,
        })
        .fold(gaps, u16::saturating_add);
    let fills = columns
        .iter()
        .filter(|c| matches!(c.width, ColumnWidth::Fill { .. }))
        .count();
    let rest = width.saturating_sub(fixed);
    let fills_u16 = u16::try_from(fills).unwrap_or(u16::MAX).max(1);
    let share = rest / fills_u16;
    let mut extra = rest % fills_u16;
    columns
        .iter()
        .map(|c| match c.width {
            ColumnWidth::Fixed(w) => w,
            ColumnWidth::Fill { min } => {
                let bonus = u16::from(extra > 0);
                extra = extra.saturating_sub(1);
                (share + bonus).max(min)
            }
        })
        .collect()
}

/// Draws a list model into an area, from the scroll offset down.
pub(crate) fn render(model: &ListModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let style = Style::new().fg(theme.palette.fg).bg(theme.palette.bg);
    buf.set_style(area, style);
    let widths = column_widths(&model.columns, area.width);
    let rows = model
        .rows
        .iter()
        .skip(model.scroll)
        .take(usize::from(area.height));
    for (y, row) in (area.y..).zip(rows) {
        let mut x = area.x;
        for ((column, &w), cell) in model.columns.iter().zip(&widths).zip(&row.cells) {
            if x >= area.right() {
                break;
            }
            let w = w.min(area.right() - x);
            let text = text::fit(&cell.text, usize::from(w), column.align);
            buf.set_stringn(x, y, text, usize::from(w), style);
            x = x.saturating_add(w + GAP);
        }
    }
}
