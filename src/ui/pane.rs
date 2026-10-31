//! Renders a pane model. Any popup pane produces one, so this renderer
//! knows no pane by name.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};

use crate::app::models::{Cell, CellStyle, Column, PaneModel, Row, TreeNode};
use crate::theme::{Palette, Theme};
use crate::ui::list::{self, Window};
use crate::ui::{markdown, style, text};

/// The cells of indent per tree depth.
const INDENT: usize = 2;

/// Draws a pane model into an area. `cursor` and `scroll` come from the
/// pane state.
pub(crate) fn render(
    model: &PaneModel,
    cursor: usize,
    scroll: usize,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return;
    }
    match model {
        PaneModel::Table { columns, rows } => {
            render_table(columns, rows, cursor, scroll, area, buf, theme);
        }
        PaneModel::Lines { lines, wrap } => {
            let lines: Vec<Line<'static>> = lines
                .iter()
                .flat_map(|cells| expand(cells, area.width, &theme.palette))
                .collect();
            render_lines(lines, *wrap, scroll, area, buf, theme);
        }
        PaneModel::Tree { nodes } => render_tree(nodes, cursor, scroll, area, buf, theme),
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
    cursor: usize,
    scroll: usize,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let p = &theme.palette;
    let header_style = Style::new()
        .bg(p.bg)
        .fg(p.fg_mute)
        .add_modifier(Modifier::BOLD);
    buf.set_style(Rect::new(area.x, area.y, area.width, 1), header_style);
    let widths = list::column_widths(columns, area.width.saturating_sub(list::GUTTER));
    let mut x = list::columns_x(area);
    for (column, &w) in columns.iter().zip(&widths) {
        if x >= area.right() {
            break;
        }
        let w = w.min(area.right() - x);
        let title = text::fit(&column.title.to_uppercase(), usize::from(w), column.align);
        buf.set_stringn(x, area.y, title, usize::from(w), header_style);
        x = x.saturating_add(w + list::GAP);
    }
    let body = Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(1),
    );
    list::render_rows(
        columns,
        rows,
        Window {
            cursor: Some(cursor),
            scroll,
        },
        body,
        buf,
        theme,
    );
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
) {
    let p = &theme.palette;
    buf.set_style(area, Style::new().bg(p.bg).fg(p.fg));
    let text_width = area.width.saturating_sub(list::GUTTER);
    let mut y = area.y;
    for (index, node) in nodes.iter().enumerate().skip(scroll) {
        if y >= area.bottom() {
            break;
        }
        let selected = index == cursor;
        let base = if selected {
            Style::new().bg(p.surface).fg(p.fg)
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
        for (row, line) in lines.into_iter().enumerate() {
            if y >= area.bottom() {
                break;
            }
            buf.set_style(Rect::new(area.x, y, area.width, 1), base);
            if selected {
                buf.set_string(area.x, y, "▌", base.fg(p.primary));
            }
            let lead = if row == 0 {
                prefix.clone()
            } else {
                " ".repeat(usize::from(prefix_width))
            };
            let mut spans = vec![Span::styled(lead, base.fg(p.fg_mute))];
            spans.extend(line.spans.into_iter().map(|s| {
                let style = base.patch(s.style);
                s.style(style)
            }));
            let rect = Rect::new(list::columns_x(area), y, text_width, 1);
            Line::from(spans).render(rect, buf);
            y += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testdata;

    fn draw(model: &PaneModel, cursor: usize, width: u16, height: u16) -> String {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        render(model, cursor, 0, area, &mut buf, &testdata::ansi16());
        (0..height)
            .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
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
