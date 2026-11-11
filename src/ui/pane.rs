//! Renders a pane model. Any popup pane produces one, so this renderer
//! knows no pane by name.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::app::measure;
use crate::app::models::{Cell, Column, PaneModel, Row, TreeNode};
use crate::bus::Hit;
use crate::theme::{Palette, Theme, punk};
use crate::ui::hits::HitMap;
use crate::ui::list::{self, Window};
use crate::ui::{markdown, style};

/// The part of a pane that scrolls, as drawn: the area of the rows that
/// scroll, the rows of the content, and the first row that shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScrollView {
    pub area: Rect,
    pub rows: usize,
    pub scroll: usize,
}

impl ScrollView {
    /// The view of `rows` rows in `area` at `scroll`. A scroll past the
    /// last full page counts as the last full page.
    fn new(area: Rect, rows: usize, scroll: usize) -> Self {
        let last = rows.saturating_sub(usize::from(area.height));
        Self {
            area,
            rows,
            scroll: scroll.min(last),
        }
    }

    /// True when the content has more rows than the area.
    pub(crate) fn scrolls(&self) -> bool {
        self.rows > usize::from(self.area.height)
    }
}

/// Draws a pane model into an area. `cursor` and `scroll` come from the
/// pane state. Each drawn table row and tree node is a `Hit::PaneRow`
/// with its index in the model. Returns the view of the rows that
/// scroll, `None` for an empty area.
pub(crate) fn render(
    model: &PaneModel,
    cursor: usize,
    scroll: usize,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    hits: &mut HitMap,
) -> Option<ScrollView> {
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return None;
    }
    let view = match model {
        PaneModel::Table { columns, rows } => {
            let window = Window {
                cursor: Some(cursor),
                scroll,
                hit: Hit::PaneRow,
            };
            let body = render_table(columns, rows, window, area, buf, theme, hits);
            ScrollView::new(body, rows.len(), scroll)
        }
        PaneModel::Lines { lines, wrap } => {
            let lines = pane_lines(lines, *wrap, area.width, &theme.palette);
            let view = ScrollView::new(area, lines.len(), scroll);
            render_lines(lines, scroll, area, buf, theme);
            view
        }
        PaneModel::Tree { nodes } => {
            let rows = render_tree(nodes, cursor, scroll, area, buf, theme, hits);
            ScrollView::new(area, rows, scroll)
        }
    };
    Some(view)
}

/// The display lines of one line of cells. A markdown or HTML cell becomes
/// the rendered body, wrapped to the width after the cells before it. The
/// next rows of the body are indented under it.
fn expand(cells: &[Cell], width: u16, palette: &Palette) -> Vec<Line<'static>> {
    let Some(at) = measure::markup_cell(cells) else {
        return vec![style::line(cells, palette)];
    };
    let mut first = style::line(&cells[..at], palette);
    let indent = first.width();
    let body_width = width.saturating_sub(u16::try_from(indent).unwrap_or(u16::MAX));
    let source = measure::source(&cells[at]);
    let mut body = markdown::render(source, body_width, palette).into_iter();
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

/// A header row with the column titles, then the rows. Returns the area
/// of the rows.
fn render_table(
    columns: &[Column],
    rows: &[Row],
    window: Window,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    hits: &mut HitMap,
) -> Rect {
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
    body
}

/// The screen rows of the lines of a pane of lines at `width`: the
/// display lines of each line, each wrapped by `measure::wrap_ranges`
/// when `wrap` is true. `measure::display_rows` counts the same rows.
fn pane_lines(
    lines: &[Vec<Cell>],
    wrap: bool,
    width: u16,
    palette: &Palette,
) -> Vec<Line<'static>> {
    let lines = lines.iter().flat_map(|cells| expand(cells, width, palette));
    if wrap {
        lines.flat_map(|line| wrap_line(line, width)).collect()
    } else {
        lines.collect()
    }
}

/// The rows of a display line wrapped at word boundaries to `width`.
/// Each piece of a row keeps the style of its span.
fn wrap_line(line: Line<'static>, width: u16) -> Vec<Line<'static>> {
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let ranges = measure::wrap_ranges(&text, usize::from(width));
    if ranges.len() == 1 {
        return vec![line];
    }
    ranges
        .into_iter()
        .map(|range| {
            let mut at = 0;
            let mut spans = Vec::new();
            for span in &line.spans {
                let end = at + span.content.len();
                let (from, to) = (range.start.max(at), range.end.min(end));
                if from < to {
                    spans.push(Span::styled(
                        span.content[from - at..to - at].to_owned(),
                        span.style,
                    ));
                }
                at = end;
            }
            Line::from(spans).style(line.style)
        })
        .collect()
}

/// Rows of text from the scroll offset down.
pub(crate) fn render_lines(
    lines: Vec<Line<'static>>,
    scroll: usize,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
) {
    let p = &theme.palette;
    Paragraph::new(lines)
        .style(Style::new().bg(p.bg).fg(p.fg))
        .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0))
        .render(area, buf);
}

/// The nodes of a tree with indent and an expand marker, from the scroll
/// offset (in rows) down. A node with a markdown body takes one row per
/// body line, so the first node can start in the middle of its rows. A
/// scroll past the last full page shows the last full page. The cursor
/// node gets the `▌` marker. Returns the rows of all the nodes.
fn render_tree(
    nodes: &[TreeNode],
    cursor: usize,
    scroll: usize,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    hits: &mut HitMap,
) -> usize {
    let p = &theme.palette;
    buf.set_style(area, Style::new().bg(p.bg).fg(p.fg));
    let selection = theme.selected_row();
    let text_width = measure::tree_text_width(area.width, selection.shift);
    let total: usize = nodes
        .iter()
        .map(|node| measure::node_rows(node, text_width))
        .sum();
    let mut skip = scroll.min(total.saturating_sub(usize::from(area.height)));
    let mut y = area.y;
    for (index, node) in nodes.iter().enumerate() {
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
        let indent = " ".repeat(measure::TREE_INDENT * usize::from(node.depth));
        let prefix = format!("{indent}{marker}");
        let prefix_width = u16::try_from(measure::node_prefix(node.depth)).unwrap_or(text_width);
        let lines = expand(&node.cells, text_width.saturating_sub(prefix_width), p);
        if skip >= lines.len() {
            skip -= lines.len();
            continue;
        }
        let top = y;
        for (row, line) in lines
            .into_iter()
            .enumerate()
            .skip(std::mem::take(&mut skip))
        {
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
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::models::CellStyle;
    use crate::ui::testdata;
    use ratatui::style::Color;

    fn draw(model: &PaneModel, cursor: usize, width: u16, height: u16) -> String {
        draw_at(model, cursor, 0, width, height)
    }

    /// Draws a pane model at `scroll` and returns the screen rows.
    fn draw_at(model: &PaneModel, cursor: usize, scroll: usize, width: u16, height: u16) -> String {
        let area = Rect::new(0, 0, width, height);
        let mut buf = Buffer::empty(area);
        let mut hits = HitMap::default();
        render(
            model,
            cursor,
            scroll,
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

    /// A tree of a header and a body of five rows, then a second
    /// header.
    fn tall_tree() -> PaneModel {
        let node = |depth, text: &str, style| TreeNode {
            depth,
            expanded: None,
            cells: vec![testdata::cell(text, style)],
        };
        PaneModel::Tree {
            nodes: vec![
                node(0, "header one", CellStyle::Info),
                node(1, "a\n\nb\n\nc", CellStyle::Markdown),
                node(0, "header two", CellStyle::Info),
            ],
        }
    }

    #[test]
    fn a_tree_scrolls_by_rows_into_a_node() {
        let rows: Vec<String> = draw_at(&tall_tree(), 1, 3, 20, 3)
            .lines()
            .map(|row| row.trim_end().to_owned())
            .collect();
        assert_eq!(
            rows,
            ["▌    b", "▌", "▌    c"],
            "rows 3 to 5: the end of the body"
        );
        let hits = hits(&tall_tree(), 3, 20, 3);
        assert_eq!(hits.at(0, 0), Some(Hit::PaneRow(1)));
        assert_eq!(hits.at(0, 2), Some(Hit::PaneRow(1)));
    }

    #[test]
    fn a_tree_scroll_past_the_end_shows_the_last_page() {
        let rows: Vec<String> = draw_at(&tall_tree(), 0, 99, 20, 3)
            .lines()
            .map(|row| row.trim_end().to_owned())
            .collect();
        assert_eq!(rows, ["", "     c", "   header two"]);
    }

    #[test]
    fn the_wrapped_lines_are_the_rows_that_app_counts() {
        let plain = |text: &str| vec![testdata::cell(text, CellStyle::Normal)];
        let markdown = |text: &str| vec![testdata::cell(text, CellStyle::Markdown)];
        let lines = vec![
            plain("3 unsent codeview comments · space r to review"),
            Vec::new(),
            markdown("## Why\nThe sidebar must open on the description of the pull request."),
            markdown("```\nlet a_long_name = a_function_with_a_long_name(argument_one);\n```"),
            vec![
                testdata::cell("by ", CellStyle::Dim),
                testdata::cell("a body after a cell that wraps", CellStyle::Markdown),
                testdata::cell(" (end)", CellStyle::Dim),
            ],
            markdown("| a | table |\n|---|---|\n| with a cell that is wide | x |"),
            plain("日本語のテキストは二つのセルを使う"),
        ];
        let palette = testdata::pierre().palette;
        for width in [1, 7, 12, 20, 33, 76] {
            let drawn = pane_lines(&lines, true, width, &palette);
            assert_eq!(
                drawn.len(),
                measure::display_rows(&lines, true, width),
                "width {width}"
            );
            assert!(
                drawn
                    .iter()
                    .all(|line| line.width() <= usize::from(width).max(2)),
                "width {width}: no row is wider than the pane"
            );
            let unwrapped = pane_lines(&lines, false, width, &palette);
            assert_eq!(
                unwrapped.len(),
                measure::display_rows(&lines, false, width),
                "width {width} without wrap"
            );
        }
    }

    #[test]
    fn a_wrapped_line_keeps_the_styles_of_its_spans() {
        let p = testdata::pierre().palette;
        let line = style::line(
            &[
                testdata::cell("alice ", CellStyle::Info),
                testdata::cell("wrote a long line", CellStyle::Normal),
            ],
            &p,
        );
        let rows = wrap_line(line, 10);
        let texts: Vec<String> = rows.iter().map(ToString::to_string).collect();
        assert_eq!(texts, ["alice ", "wrote a ", "long line"]);
        assert_eq!(rows[0].spans[0].style.fg, Some(p.info));
        assert_eq!(rows[1].spans[0].style.fg, Some(p.fg));
    }

    #[test]
    fn a_merge_badge_fills_its_cells_with_the_pulse_color() {
        let model = PaneModel::Lines {
            lines: [0, 128, 255]
                .into_iter()
                .map(|mix| vec![testdata::cell(" MERGING ", CellStyle::MergeBadge(mix))])
                .collect(),
            wrap: true,
        };
        let pierre = testdata::pierre();
        let ansi16 = testdata::ansi16();
        let cases = [
            (
                &pierre,
                [
                    pierre.palette.success,
                    Color::Rgb(0x52, 0x97, 0xbe),
                    pierre.palette.purple,
                ],
            ),
            (&ansi16, [Color::Green, Color::Magenta, Color::Magenta]),
        ];
        for (theme, colors) in cases {
            let area = Rect::new(0, 0, 12, 3);
            let mut buf = Buffer::empty(area);
            render(&model, 0, 0, area, &mut buf, theme, &mut HitMap::default());
            for (y, color) in (0..).zip(colors) {
                for x in 0..9 {
                    assert_eq!(buf[(x, y)].bg, color, "{x}, {y}");
                    assert_eq!(buf[(x, y)].fg, theme.palette.bg, "{x}, {y}");
                }
                assert_ne!(buf[(9, y)].bg, color, "the badge ends at its text");
            }
        }
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
