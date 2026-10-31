//! Renders a pane model. Any popup pane produces one, so this renderer
//! knows no pane by name.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};

use crate::app::models::{Column, PaneModel, Row, TreeNode};
use crate::theme::Theme;
use crate::ui::list::{self, Window};
use crate::ui::{style, text};

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
                .map(|cells| style::line(cells, &theme.palette))
                .collect();
            render_lines(lines, *wrap, scroll, area, buf, theme);
        }
        PaneModel::Tree { nodes } => render_tree(nodes, cursor, scroll, area, buf, theme),
    }
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
/// offset down. The cursor node gets the `▌` marker.
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
    let visible = nodes
        .iter()
        .enumerate()
        .skip(scroll)
        .take(usize::from(area.height));
    for (y, (index, node)) in (area.y..).zip(visible) {
        let selected = index == cursor;
        let base = if selected {
            Style::new().bg(p.surface).fg(p.fg)
        } else {
            Style::new().bg(p.bg).fg(p.fg)
        };
        buf.set_style(Rect::new(area.x, y, area.width, 1), base);
        if selected {
            buf.set_string(area.x, y, "▌", base.fg(p.primary));
        }
        let marker = match node.expanded {
            Some(true) => "▾ ",
            Some(false) => "▸ ",
            None => "  ",
        };
        let indent = " ".repeat(INDENT * usize::from(node.depth));
        let mut spans = vec![Span::styled(
            format!("{indent}{marker}"),
            base.fg(p.fg_mute),
        )];
        spans.extend(style::line(&node.cells, p).spans.into_iter().map(|s| {
            let style = base.patch(s.style);
            s.style(style)
        }));
        let row = Rect::new(
            list::columns_x(area),
            y,
            area.width.saturating_sub(list::GUTTER),
            1,
        );
        Line::from(spans).render(row, buf);
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
}
