//! Renders a list model. Any tab produces one, so this renderer knows no
//! tab by name.
//!
//! Each row starts with a one-cell gutter that holds the cursor marker
//! `▌`. The first column gets the tree indent and glyph of the row. Stack
//! headers and separators span the full width. Empty rows below the list
//! get the texture.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};

use crate::app::models::{Cell, CellStyle, Column, ColumnWidth, ListModel, Row, RowItem, RowStyle};
use crate::bus::Hit;
use crate::theme::{Theme, punk};
use crate::ui::hits::HitMap;
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
        RowStyle::Dim => base.fg(p.fg_mute),
        RowStyle::Header => base.fg(p.fg_dim).add_modifier(Modifier::BOLD),
    };
    if selected || row.style == RowStyle::Selected {
        base.patch(theme.selected_row().patch)
            .add_modifier(Modifier::BOLD)
    } else {
        base
    }
}

/// The style of a cell in a row. Plain text in a dim row turns grey; markers keep their color.
fn cell_style(cell: &Cell, row: &Row, theme: &Theme) -> Style {
    let semantic = match (row.style, cell.style) {
        (RowStyle::Dim, CellStyle::Normal) => CellStyle::Dim,
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

/// The cells at the right end of every row that the selection tail and
/// the selection shift of the theme keep free.
fn tail_width(theme: &Theme) -> u16 {
    let selection = theme.selected_row();
    let tail = if selection.tail.is_some() { 2 } else { 0 };
    tail + selection.shift
}

/// The width of the columns of a list `width` cells wide: the width
/// without the cursor gutter and the selection tail.
pub(crate) fn content_width(width: u16, theme: &Theme) -> u16 {
    width.saturating_sub(GUTTER + tail_width(theme))
}

/// The style with the text color `fg`, or the style as it is.
fn with_fg(style: Style, fg: Option<Color>) -> Style {
    fg.map_or(style, |fg| style.fg(fg))
}

/// What every row of one draw shares.
struct Frame<'a> {
    columns: &'a [Column],
    widths: &'a [u16],
    area: Rect,
    theme: &'a Theme,
}

/// Draws one row at `y`.
fn render_row(frame: &Frame<'_>, row: &Row, y: u16, selected: bool, buf: &mut Buffer) {
    let Frame {
        columns,
        widths,
        area,
        theme,
    } = *frame;
    let line = Rect::new(area.x, y, area.width, 1);
    let base = row_style(row, selected, theme);
    buf.set_style(line, base);
    let selection = theme.selected_row();
    let banded = selected || row.style == RowStyle::Selected;
    // A band can force one text color on its cells.
    let fg = if banded { selection.cell_fg } else { None };
    if banded && let Some([(first, first_style), (second, second_style)]) = selection.tail {
        buf.set_string(line.right().saturating_sub(2), y, first, first_style);
        buf.set_string(line.right() - 1, y, second, second_style);
    }
    if selected {
        let marker = Style::new().patch(selection.patch).fg(selection.marker_fg);
        let marker = selection.marker_bg.map_or(marker, |bg| marker.bg(bg));
        buf.set_string(area.x, y, selection.marker, marker);
    }
    let x0 = area.x + GUTTER;
    let right = area.right().saturating_sub(tail_width(theme)).max(x0);
    match row.item {
        RowItem::Separator => {
            let width = right.saturating_sub(x0);
            buf.set_span(
                x0,
                y,
                &theme.separator(usize::from(width), base.fg(theme.palette.surface)),
                width,
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
                let style = with_fg(
                    base.patch(cell_style(cell, row, theme).remove_modifier(Modifier::BOLD)),
                    fg,
                );
                let (nx, _) = buf.set_stringn(x, y, &cell.text, usize::from(right - x), style);
                x = nx;
            }
            return;
        }
        RowItem::Pr(_) => {}
    }
    let mut x = x0;
    for (i, ((column, &w), cell)) in columns.iter().zip(widths).zip(&row.cells).enumerate() {
        if x >= right {
            break;
        }
        let w = w.min(right - x);
        let mut cell_x = x;
        let mut cell_w = usize::from(w);
        if i == 0 {
            let prefix = tree_prefix(row);
            let glyph_style = with_fg(base.fg(theme.palette.fg_mute), fg);
            let (px, _) = buf.set_stringn(cell_x, y, &prefix, cell_w, glyph_style);
            cell_w = cell_w.saturating_sub(usize::from(px - cell_x));
            cell_x = px;
        }
        let fitted = text::fit(&cell.text, cell_w, column.align);
        let style = with_fg(base.patch(cell_style(cell, row, theme)), fg);
        buf.set_stringn(cell_x, y, fitted, cell_w, style);
        x = x.saturating_add(w + GAP);
    }
}

/// Draws a list model into an area, from the scroll offset down. Each
/// drawn row is a `Hit::Row` with its index in the model.
pub(crate) fn render(
    model: &ListModel,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    hits: &mut HitMap,
) {
    render_rows(
        &model.columns,
        &model.rows,
        Window {
            cursor: Some(model.cursor),
            scroll: model.scroll,
            hit: Hit::Row,
        },
        area,
        buf,
        theme,
        hits,
    );
}

/// The cursor and the scroll offset of a list of rows, and the hit of a
/// row.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Window {
    /// The selected row, if rows can be selected.
    pub cursor: Option<usize>,
    /// The index of the first visible row.
    pub scroll: usize,
    /// The hit of the row at an index in the rows.
    pub hit: fn(usize) -> Hit,
}

/// The x offset of the first column, after the gutter.
pub(crate) fn columns_x(area: Rect) -> u16 {
    area.x + GUTTER
}

/// Draws rows with columns into an area. The list and the pane tables
/// share it. Each drawn row is the hit of `window` for its index.
pub(crate) fn render_rows(
    columns: &[Column],
    rows: &[Row],
    window: Window,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let area = area.intersection(buf.area);
    buf.set_style(area, Style::new().fg(theme.palette.fg).bg(theme.palette.bg));
    for (x, y) in
        (area.top()..area.bottom()).flat_map(|y| (area.left()..area.right()).map(move |x| (x, y)))
    {
        buf[(x, y)].set_symbol(" ");
    }
    let widths = column_widths(columns, content_width(area.width, theme));
    let visible = rows
        .iter()
        .enumerate()
        .skip(window.scroll)
        .take(usize::from(area.height));
    let frame = Frame {
        columns,
        widths: &widths,
        area,
        theme,
    };
    let mut drawn = 0;
    let shift = theme.selected_row().shift;
    for (y, (index, row)) in (area.y..).zip(visible) {
        let selected = window.cursor == Some(index);
        render_row(&frame, row, y, selected, buf);
        if selected || row.style == RowStyle::Selected {
            // The band jumps out: its text moves into the free cells.
            let x0 = area.x + GUTTER;
            let right = area.right().saturating_sub(tail_width(theme)).max(x0);
            let end = right.saturating_add(shift).min(area.right());
            let content = Rect::new(x0, y, end.saturating_sub(x0), 1);
            punk::shift_right(buf, content, shift, row_style(row, selected, theme));
        }
        hits.push(Rect::new(area.x, y, area.width, 1), (window.hit)(index));
        drawn += 1;
    }
    let empty = Rect::new(
        area.x,
        area.y + drawn,
        area.width,
        area.height.saturating_sub(drawn),
    );
    theme.fill(empty, buf, 0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::models::Align;
    use crate::ui::testdata;

    fn draw(model: &ListModel, width: u16, height: u16, theme: &Theme) -> Buffer {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        render(model, area, &mut buf, theme, &mut HitMap::default());
        buf
    }

    #[test]
    fn each_drawn_row_is_a_hit_with_its_index() {
        let mut model = testdata::mine_list();
        model.scroll = 2;
        let area = Rect::new(0, 3, 100, 4);
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 10));
        let mut hits = HitMap::default();
        render(&model, area, &mut buf, &testdata::ansi16(), &mut hits);
        assert_eq!(hits.at(0, 3), Some(Hit::Row(2)));
        assert_eq!(hits.at(50, 4), Some(Hit::Row(3)));
        assert_eq!(hits.at(99, 5), Some(Hit::Row(4)));
        assert_eq!(hits.at(0, 2), None, "above the list");
        assert_eq!(hits.at(0, 6), None, "no row is drawn here");
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
        model.cursor = 2;
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
        // Row 3 has a red CI glyph.
        let ci = lines(&buf)
            .lines()
            .nth(3)
            .and_then(|l| l.chars().position(|c| c == '✗'))
            .unwrap();
        assert_eq!(buf[(u16::try_from(ci).unwrap(), 3)].fg, p.danger);
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

    #[test]
    fn punk_selected_row_is_a_band() {
        let theme = testdata::punk();
        let p = theme.palette;
        let buf = draw(&testdata::mine_list(), 100, 9, &theme);
        let row = lines(&buf).lines().nth(1).unwrap().to_owned();
        // The text moves one cell to the right.
        assert!(row.starts_with("▶ ┌ feat(ui)"), "{row}");
        assert!(row.ends_with("◤◤"), "{row}");
        // The marker is black on white.
        assert_eq!(buf[(0, 1)].fg, p.bg);
        assert_eq!(buf[(0, 1)].bg, p.fg);
        // Every cell on the band is black, colored markers too.
        for x in 1..98 {
            assert_eq!(buf[(x, 1)].bg, p.primary, "x {x}");
            assert_eq!(buf[(x, 1)].fg, p.bg, "x {x}");
        }
        // The blue card ends on the white card, and the white card ends.
        assert_eq!(buf[(98, 1)].fg, p.primary);
        assert_eq!(buf[(98, 1)].bg, p.fg);
        assert_eq!(buf[(99, 1)].fg, p.fg);
        assert_eq!(buf[(99, 1)].bg, p.bg);
        // Other rows keep the tail cells and the shift cell free.
        for x in 97..100 {
            assert_eq!(buf[(x, 4)].symbol(), " ", "x {x}");
        }
    }

    #[test]
    fn punk_selected_row_keeps_the_last_character() {
        let mut model = testdata::mine_list();
        model.columns.truncate(2);
        for row in &mut model.rows {
            row.cells.truncate(2);
        }
        assert_eq!(model.columns[1].align, Align::Right);
        let text = lines(&draw(&model, 40, 3, &testdata::punk()));
        let row = text.lines().nth(1).unwrap();
        assert!(row.ends_with("#103◤◤"), "{row}");
    }
}
