//! Renders a pane model. Any popup pane produces one, so this renderer
//! knows no pane by name.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};

use crate::app::models::{Cell, CellStyle, Column, PaneModel, Row, TreeNode};
use crate::bus::Hit;
use crate::theme::{Palette, Theme, punk};
use crate::ui::hits::HitMap;
use crate::ui::list::{self, Window};
use crate::ui::{markdown, style, text};

/// The cells of indent per tree depth.
const INDENT: usize = 2;

/// Draws a pane model into an area. `cursor` and `scroll` come from the
/// pane state. Each drawn table row and tree node is a `Hit::PaneRow`
/// with its index in the model.
pub(crate) fn render(
    model: &PaneModel,
    cursor: usize,
    scroll: usize,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return;
    }
    match model {
        PaneModel::Table { columns, rows } => {
            let window = Window {
                cursor: Some(cursor),
                scroll,
                hit: Hit::PaneRow,
            };
            render_table(columns, rows, window, area, buf, theme, hits);
        }
        PaneModel::Lines { lines, wrap } => {
            let lines: Vec<Line<'static>> = lines
                .iter()
                .flat_map(|cells| expand(cells, area.width, &theme.palette))
                .collect();
            render_lines(lines, *wrap, scroll, area, buf, theme);
        }
        PaneModel::Tree { nodes } => render_tree(nodes, cursor, scroll, area, buf, theme, hits),
    }
}

/// The display lines of one line of cells. A markdown cell becomes the
/// rendered body, wrapped to the width after the cells before it. The
/// next rows of the body are indented under it.
fn expand(cells: &[Cell], width: u16, palette: &Palette) -> Vec<Line<'static>> {
    let Some(at) = cells.iter().position(|c| c.style == CellStyle::Markdown) else {
        return vec![style::line(cells, palette)];
    };
    let mut first = style::line(&cells[..at], palette);
    let indent = first.width();
    let body_width = width.saturating_sub(u16::try_from(indent).unwrap_or(u16::MAX));
    let mut body = markdown::render(&cells[at].text, body_width, palette).into_iter();
    if let Some(line) = body.next() {
        first.spans.extend(line.spans);
    }
    let mut out = vec![first];
    out.extend(body.map(|mut line| {
        line.spans.insert(0, Span::raw(" ".repeat(indent)));
        line
    }));
    if let Some(last) = out.last_mut() {
        last.spans
            .extend(style::line(&cells[at + 1..], palette).spans);
    }
    out
}

/// A header row with the column titles, then the rows.
fn render_table(
    columns: &[Column],
    rows: &[Row],
    window: Window,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    hits: &mut HitMap,
) {
    list::render_header(
        columns,
        Rect::new(area.x, area.y, area.width, area.height.min(1)),
        buf,
        theme,
    );
    let body = Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(1),
    );
    list::render_rows(columns, rows, window, body, buf, theme, hits);
}

/// Lines of text, wrapped at word boundaries when `wrap` is true, from the
/// scroll offset down.
pub(crate) fn render_lines(
    lines: Vec<Line<'static>>,
    wrap: bool,
    scroll: usize,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let p = &theme.palette;
    let mut paragraph = Paragraph::new(lines)
        .style(Style::new().bg(p.bg).fg(p.fg))
        .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0));
    if wrap {
        paragraph = paragraph.wrap(Wrap { trim: false });
    }
    paragraph.render(area, buf);
}

/// The nodes of a tree with indent and an expand marker, from the scroll
/// offset (in nodes) down. A node with a markdown body takes one row per
/// body line. The cursor node gets the `▌` marker.
fn render_tree(
    nodes: &[TreeNode],
    cursor: usize,
    scroll: usize,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let p = &theme.palette;
    buf.set_style(area, Style::new().bg(p.bg).fg(p.fg));
    let selection = theme.selected_row();
    let text_width = area
        .width
        .saturating_sub(list::GUTTER)
        .saturating_sub(selection.shift);
    let mut y = area.y;
    for (index, node) in nodes.iter().enumerate().skip(scroll) {
        if y >= area.bottom() {
            break;
        }
        let selected = index == cursor;
        let base = if selected {
            Style::new().bg(p.bg).fg(p.fg).patch(selection.patch)
        } else {
            Style::new().bg(p.bg).fg(p.fg)
        };
        let marker = match node.expanded {
            Some(true) => "▾ ",
            Some(false) => "▸ ",
            None => "  ",
        };
        let prefix = format!("{}{marker}", " ".repeat(INDENT * usize::from(node.depth)));
        let prefix_width = u16::try_from(text::width(&prefix)).unwrap_or(text_width);
        let lines = expand(&node.cells, text_width.saturating_sub(prefix_width), p);
        let top = y;
        for (row, line) in lines.into_iter().enumerate() {
            if y >= area.bottom() {
                break;
            }
            buf.set_style(Rect::new(area.x, y, area.width, 1), base);
            if selected {
                let marker = base.fg(selection.marker_fg);
                let marker = selection.marker_bg.map_or(marker, |bg| marker.bg(bg));
                buf.set_string(area.x, y, selection.marker, marker);
            }
            let lead = if row == 0 {
                prefix.clone()
            } else {
                " ".repeat(usize::from(prefix_width))
            };
            // A band can force one text color on the selected node.
            let fg = if selected { selection.cell_fg } else { None };
            let over = |style: Style| fg.map_or(style, |fg| style.fg(fg));
            let mut spans = vec![Span::styled(lead, over(base.fg(p.fg_mute)))];
            spans.extend(line.spans.into_iter().map(|s| {
                let style = over(base.patch(s.style));
                s.style(style)
            }));
            let rect = Rect::new(list::columns_x(area), y, text_width, 1);
            Line::from(spans).render(rect, buf);
            if selected {
                // The band jumps out: its text moves into the free cells.
                let width = (text_width + selection.shift).min(area.right().saturating_sub(rect.x));
                let content = Rect::new(rect.x, y, width, 1);
                punk::shift_right(buf, content, selection.shift, base);
            }
            y += 1;
        }
        hits.push(
            Rect::new(area.x, top, area.width, y - top),
            Hit::PaneRow(index),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testdata;

    fn draw(model: &PaneModel, cursor: usize, width: u16, height: u16) -> String {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        let mut hits = HitMap::default();
        render(
            model,
            cursor,
            0,
            area,
            &mut buf,
            &testdata::ansi16(),
            &mut hits,
        );
        (0..height)
            .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The hit map of a pane model drawn at `scroll`.
    fn hits(model: &PaneModel, scroll: usize, width: u16, height: u16) -> HitMap {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        let mut hits = HitMap::default();
        render(
            model,
            0,
            scroll,
            area,
            &mut buf,
            &testdata::ansi16(),
            &mut hits,
        );
        hits
    }

    #[test]
    fn table_rows_are_hits_below_the_header() {
        let hits = hits(&testdata::checks_pane(), 1, 60, 6);
        assert_eq!(hits.at(5, 0), None, "the header row");
        assert_eq!(hits.at(5, 1), Some(Hit::PaneRow(1)));
        assert_eq!(hits.at(59, 2), Some(Hit::PaneRow(2)));
        assert_eq!(hits.at(5, 3), None, "no more rows");
    }

    #[test]
    fn tree_nodes_are_hits_over_all_their_rows() {
        let PaneModel::Tree { mut nodes } = testdata::tree_pane() else {
            unreachable!()
        };
        nodes.insert(
            0,
            TreeNode {
                depth: 0,
                expanded: None,
                cells: vec![testdata::cell(
                    "a long body that wraps over more than one row of the pane",
                    CellStyle::Markdown,
                )],
            },
        );
        let hits = hits(&PaneModel::Tree { nodes }, 0, 30, 8);
        assert_eq!(hits.at(0, 0), Some(Hit::PaneRow(0)));
        let second = (1..8)
            .find(|&y| hits.at(0, y) != Some(Hit::PaneRow(0)))
            .unwrap();
        assert!(second > 1, "the first node takes more than one row");
        assert_eq!(hits.at(29, second), Some(Hit::PaneRow(1)));
        assert_eq!(hits.at(0, second + 1), Some(Hit::PaneRow(2)));
    }

    #[test]
    fn lines_have_no_row_hits() {
        let hits = hits(&testdata::lines_pane(), 0, 40, 6);
        assert_eq!(hits.at(0, 0), None);
    }

    #[test]
    fn table_pane() {
        insta::assert_snapshot!(draw(&testdata::checks_pane(), 1, 60, 6));
    }

    #[test]
    fn lines_pane_wraps() {
        insta::assert_snapshot!(draw(&testdata::lines_pane(), 0, 40, 6));
    }

    #[test]
    fn tree_pane() {
        insta::assert_snapshot!(draw(&testdata::tree_pane(), 2, 60, 6));
    }

    #[test]
    fn tree_pane_with_markdown_bodies() {
        let PaneModel::Tree { mut nodes } = testdata::tree_pane() else {
            unreachable!()
        };
        nodes.insert(
            1,
            TreeNode {
                depth: 1,
                expanded: None,
                cells: vec![testdata::cell(
                    "Looks good. Please call `Transitions::prepare` before **every** draw, \
                     not only while a slide runs.",
                    CellStyle::Markdown,
                )],
            },
        );
        insta::assert_snapshot!(draw(&PaneModel::Tree { nodes }, 1, 50, 8));
    }

    #[test]
    fn lines_pane_with_a_markdown_cell() {
        let model = PaneModel::Lines {
            lines: vec![vec![
                testdata::cell("alice: ", CellStyle::Info),
                testdata::cell(
                    "the *new* layout drops the `header` row in the list",
                    CellStyle::Markdown,
                ),
            ]],
            wrap: true,
        };
        insta::assert_snapshot!(draw(&model, 0, 30, 4));
    }
}
