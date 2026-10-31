//! Renders a list model. Any tab produces one, so this renderer knows no
//! tab by name.
//!
//! Each row starts with a one-cell gutter that holds the cursor marker
//! `▌`. The first column gets the tree indent and glyph of the row. Stack
//! headers and separators span the full width. Empty rows below the list
//! get the texture.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Widget;

use crate::app::models::{Cell, CellStyle, Column, ColumnWidth, ListModel, Row, RowItem, RowStyle};
use crate::theme::Theme;
use crate::theme::texture::Texture;
use crate::ui::{style, text};

/// The space between two columns.
pub(crate) const GAP: u16 = 1;
/// The width of the cursor gutter.
pub(crate) const GUTTER: u16 = 1;
/// The cells of indent per tree depth.
const INDENT: usize = 2;

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

/// The base style of a row: its foreground and background.
fn row_style(row: &Row, selected: bool, theme: &Theme) -> Style {
    let p = &theme.palette;
    let base = Style::new().bg(p.bg).fg(p.fg);
    let base = match row.style {
        RowStyle::Normal | RowStyle::Selected => base,
        RowStyle::Dim | RowStyle::LocalOnly => base.fg(p.fg_mute),
        RowStyle::Header => base.fg(p.fg_dim).add_modifier(Modifier::BOLD),
    };
    if selected || row.style == RowStyle::Selected {
        base.bg(p.surface).add_modifier(Modifier::BOLD)
    } else {
        base
    }
}

/// The style of a cell in a row. Plain text in a dim or local-only row
/// turns grey; markers keep their color.
fn cell_style(cell: &Cell, row: &Row, theme: &Theme) -> Style {
    let semantic = match (row.style, cell.style) {
        (RowStyle::Dim | RowStyle::LocalOnly, CellStyle::Normal) => CellStyle::Dim,
        (_, s) => s,
    };
    style::cell_style(semantic, &theme.palette)
}

/// The tree prefix of a row: the indent, the glyph and one space.
fn tree_prefix(row: &Row) -> String {
    let indent = " ".repeat(INDENT * usize::from(row.depth));
    match row.glyph {
        Some(glyph) => format!("{indent}{glyph} "),
        None => indent,
    }
}

/// What every row of one draw shares.
struct Frame<'a> {
    model: &'a ListModel,
    widths: &'a [u16],
    area: Rect,
    theme: &'a Theme,
}

/// Draws one row at `y`.
fn render_row(frame: &Frame<'_>, row: &Row, y: u16, selected: bool, buf: &mut Buffer) {
    let Frame {
        model,
        widths,
        area,
        theme,
    } = *frame;
    let line = Rect::new(area.x, y, area.width, 1);
    let base = row_style(row, selected, theme);
    buf.set_style(line, base);
    if selected {
        buf.set_string(
            area.x,
            y,
            "▌",
            Style::new()
                .fg(theme.palette.primary)
                .bg(theme.palette.surface),
        );
    }
    let x0 = area.x + GUTTER;
    let right = area.right();
    match row.item {
        RowItem::Separator => {
            let width = usize::from(right.saturating_sub(x0));
            buf.set_stringn(
                x0,
                y,
                "─".repeat(width),
                width,
                base.fg(theme.palette.surface),
            );
            return;
        }
        RowItem::StackHeader { .. } => {
            let mut x = x0;
            let prefix = tree_prefix(row);
            let (px, _) = buf.set_stringn(x, y, &prefix, usize::from(right - x), base);
            x = px;
            for cell in &row.cells {
                if x >= right {
                    break;
                }
                let style =
                    base.patch(cell_style(cell, row, theme).remove_modifier(Modifier::BOLD));
                let (nx, _) = buf.set_stringn(x, y, &cell.text, usize::from(right - x), style);
                x = nx;
            }
            return;
        }
        RowItem::Pr(_) | RowItem::Local { .. } => {}
    }
    let mut x = x0;
    for (i, ((column, &w), cell)) in model.columns.iter().zip(widths).zip(&row.cells).enumerate() {
        if x >= right {
            break;
        }
        let w = w.min(right - x);
        let mut cell_x = x;
        let mut cell_w = usize::from(w);
        if i == 0 {
            let prefix = tree_prefix(row);
            let glyph_style = base.fg(theme.palette.fg_mute);
            let (px, _) = buf.set_stringn(cell_x, y, &prefix, cell_w, glyph_style);
            cell_w = cell_w.saturating_sub(usize::from(px - cell_x));
            cell_x = px;
        }
        let fitted = text::fit(&cell.text, cell_w, column.align);
        let style = base.patch(cell_style(cell, row, theme));
        buf.set_stringn(cell_x, y, fitted, cell_w, style);
        x = x.saturating_add(w + GAP);
    }
}

/// Draws a list model into an area, from the scroll offset down.
pub(crate) fn render(model: &ListModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let area = area.intersection(buf.area);
    buf.set_style(area, Style::new().fg(theme.palette.fg).bg(theme.palette.bg));
    for (x, y) in
        (area.top()..area.bottom()).flat_map(|y| (area.left()..area.right()).map(move |x| (x, y)))
    {
        buf[(x, y)].set_symbol(" ");
    }
    let widths = column_widths(&model.columns, area.width.saturating_sub(GUTTER));
    let rows = model
        .rows
        .iter()
        .enumerate()
        .skip(model.scroll)
        .take(usize::from(area.height));
    let frame = Frame {
        model,
        widths: &widths,
        area,
        theme,
    };
    let mut drawn = 0;
    for (y, (index, row)) in (area.y..).zip(rows) {
        render_row(&frame, row, y, index == model.cursor, buf);
        drawn += 1;
    }
    let empty = Rect::new(
        area.x,
        area.y + drawn,
        area.width,
        area.height.saturating_sub(drawn),
    );
    Texture::new(theme).render(empty, buf);
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;
    use crate::app::models::Align;
    use crate::ui::testdata;

    fn draw(model: &ListModel, width: u16, height: u16, theme: &Theme) -> Buffer {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        render(model, area, &mut buf, theme);
        buf
    }

    fn lines(buf: &Buffer) -> String {
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn widths_fill_the_rest() {
        let columns = vec![
            Column {
                title: String::new(),
                width: ColumnWidth::Fill { min: 10 },
                align: Align::Left,
            },
            Column {
                title: String::new(),
                width: ColumnWidth::Fixed(6),
                align: Align::Right,
            },
        ];
        assert_eq!(column_widths(&columns, 40), vec![33, 6]);
        assert_eq!(column_widths(&columns, 12), vec![10, 6]);
    }

    #[test]
    fn mine_100_pierre() {
        insta::assert_snapshot!(lines(&draw(
            &testdata::mine_list(),
            100,
            9,
            &testdata::pierre()
        )));
    }

    #[test]
    fn mine_160_pierre() {
        insta::assert_snapshot!(lines(&draw(
            &testdata::mine_list(),
            160,
            9,
            &testdata::pierre()
        )));
    }

    #[test]
    fn mine_100_ansi16() {
        insta::assert_snapshot!(lines(&draw(
            &testdata::mine_list(),
            100,
            9,
            &testdata::ansi16()
        )));
    }

    #[test]
    fn mine_160_ansi16() {
        insta::assert_snapshot!(lines(&draw(
            &testdata::mine_list(),
            160,
            9,
            &testdata::ansi16()
        )));
    }

    #[test]
    fn scrolled_list() {
        let mut model = testdata::mine_list();
        model.scroll = 2;
        model.cursor = 3;
        insta::assert_snapshot!(lines(&draw(&model, 100, 4, &testdata::ansi16())));
    }

    #[test]
    fn row_styles_map_to_colors() {
        let theme = testdata::pierre();
        let p = theme.palette;
        let buf = draw(&testdata::mine_list(), 100, 9, &theme);
        // Row 1 is the cursor row: the marker and the surface background.
        assert_eq!(buf[(0, 1)].symbol(), "▌");
        assert_eq!(buf[(0, 1)].fg, p.primary);
        assert_eq!(buf[(10, 1)].bg, p.surface);
        // Row 2 is local-only: grey title, orange marker.
        assert_eq!(buf[(6, 2)].fg, p.fg_mute);
        let flags = lines(&buf)
            .lines()
            .nth(2)
            .and_then(|l| l.chars().position(|c| c == '+'))
            .unwrap();
        assert_eq!(buf[(u16::try_from(flags).unwrap(), 2)].fg, p.orange);
        // Row 4 has a red CI glyph.
        let ci = lines(&buf)
            .lines()
            .nth(4)
            .and_then(|l| l.chars().position(|c| c == '✗'))
            .unwrap();
        assert_eq!(buf[(u16::try_from(ci).unwrap(), 4)].fg, p.danger);
        assert_ne!(buf[(10, 3)].bg, Color::Reset);
    }

    #[test]
    fn separator_spans_the_row() {
        let mut model = testdata::mine_list();
        model.rows.truncate(1);
        model.rows.push(Row {
            item: RowItem::Separator,
            cells: Vec::new(),
            depth: 0,
            glyph: None,
            style: RowStyle::Dim,
        });
        model.cursor = 9;
        let text = lines(&draw(&model, 20, 2, &testdata::ansi16()));
        assert_eq!(text.lines().nth(1), Some(" ───────────────────"));
    }
}
