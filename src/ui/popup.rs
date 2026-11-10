//! The popup frame with pane tabs: the header with the title and the
//! subtitle, the pane tabs, and the active pane below. The transitions
//! slide it in from the right and out again.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, ScrollbarState, StatefulWidget, Widget};

use crate::app::models::{PANE_TAB_GAP, PaneModel, PaneTab, pane_tab_rows};
use crate::bus::Hit;
use crate::theme::Theme;
use crate::theme::ThemeBlock;
use crate::ui::hits::HitMap;
use crate::ui::{pane, text};

/// The rows above the pane with one row of pane tabs: title, subtitle,
/// pane tabs and a rule. Each more row of pane tabs adds one.
const HEADER_ROWS: u16 = 4;

/// The content of the popup, borrowed from the overlay model.
pub(crate) struct Popup<'a> {
    pub title: &'a str,
    pub subtitle: &'a str,
    pub tabs: &'a [PaneTab],
    pub active: usize,
    pub pane: &'a PaneModel,
    pub cursor: usize,
    pub scroll: usize,
}

/// One row of pane tabs: `1 Description  2 Checks  3 Conversation`, with
/// the active tab marked. `first` is the index of the first tab of the
/// row in all the tabs. Each tab label in `area` is a `Hit::PaneTab`
/// with its index in all the tabs.
fn tabs_line(
    tabs: &[PaneTab],
    first: usize,
    active: usize,
    theme: &Theme,
    area: Rect,
    hits: &mut HitMap,
) -> Line<'static> {
    let mut spans = Vec::new();
    let mut x = area.x;
    for (i, tab) in tabs.iter().enumerate() {
        let i = first + i;
        if i > first {
            spans.push(Span::raw(PANE_TAB_GAP));
            x = x.saturating_add(u16::try_from(PANE_TAB_GAP.len()).unwrap_or(u16::MAX));
        }
        let cells = tab.cells();
        let mut tab = theme.tab(&tab.label(), i == active);
        // The skins draw one or two edge cells. Spaces fill the tab to
        // its width, so that each skin gives the rows of `pane_tab_rows`.
        let drawn = tab.iter().map(Span::width).sum::<usize>();
        tab.push(Span::raw(" ".repeat(cells.saturating_sub(drawn))));
        let width = u16::try_from(cells.max(drawn)).unwrap_or(u16::MAX);
        hits.push(
            Rect::new(x, area.y, width, 1).intersection(area),
            Hit::PaneTab(i),
        );
        x = x.saturating_add(width);
        spans.extend(tab);
    }
    Line::from(spans)
}

/// Draws the popup into its area. The whole area is a `Hit::Inside`,
/// under the hits of the pane tabs and the pane rows.
pub(crate) fn render(
    popup: &Popup<'_>,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let p = &theme.palette;
    let area = area.intersection(buf.area);
    hits.push(area, Hit::Inside);
    Clear.render(area, buf);
    theme.block("").accent(p.primary).seed(7).render(area, buf);
    let inner = ThemeBlock::inner(area);
    let inner = Rect::new(
        inner.x + 1,
        inner.y,
        inner.width.saturating_sub(2),
        inner.height,
    );
    let rows = pane_tab_rows(popup.tabs, inner.width);
    let tab_rows = u16::try_from(rows.len()).unwrap_or(u16::MAX);
    let header = HEADER_ROWS.saturating_add(tab_rows - 1);
    if inner.height < header || inner.width == 0 {
        return;
    }
    let width = usize::from(inner.width);
    buf.set_string(
        inner.x,
        inner.y,
        text::truncate(popup.title, width),
        Style::new().fg(p.fg).add_modifier(Modifier::BOLD),
    );
    buf.set_string(
        inner.x,
        inner.y + 1,
        text::truncate(popup.subtitle, width),
        Style::new().fg(p.fg_mute),
    );
    for (row, range) in (0..).zip(rows) {
        let tabs_area = Rect::new(inner.x, inner.y + 2 + row, inner.width, 1);
        let first = range.start;
        tabs_line(
            &popup.tabs[range],
            first,
            popup.active,
            theme,
            tabs_area,
            hits,
        )
        .render(tabs_area, buf);
    }
    buf.set_span(
        inner.x,
        inner.y + header - 1,
        &theme.separator(width, Style::new().fg(p.fg_mute)),
        inner.width,
    );
    let body = Rect::new(
        inner.x,
        inner.y + header,
        inner.width,
        inner.height - header,
    );
    hits.push_text(body);
    let view = pane::render(
        popup.pane,
        popup.cursor,
        popup.scroll,
        body,
        buf,
        theme,
        hits,
    );
    if let Some(view) = view.filter(pane::ScrollView::scrolls) {
        render_scrollbar(&view, body.right(), buf, theme);
    }
}

/// The scrollbar of a pane that scrolls, in the right padding column at
/// `x`, beside the rows that scroll. The mouse does not use it.
fn render_scrollbar(view: &pane::ScrollView, x: u16, buf: &mut Buffer, theme: &Theme) {
    let area = Rect::new(x, view.area.y, 1, view.area.height).intersection(buf.area);
    let height = usize::from(view.area.height);
    // One position per first row: the thumb reaches the end of the
    // track on the last full page.
    let mut state = ScrollbarState::new(view.rows - height + 1)
        .position(view.scroll)
        .viewport_content_length(height);
    theme.scrollbar().render(area, buf, &mut state);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testdata;

    /// The text of row `y` of a buffer.
    fn row(buf: &Buffer, y: u16) -> String {
        (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
    }

    /// The x of the first cell of `text` in row `y`.
    fn find(buf: &Buffer, y: u16, text: &str) -> u16 {
        let line = row(buf, y);
        let at = line.find(text).unwrap();
        u16::try_from(line[..at].chars().count()).unwrap()
    }

    #[test]
    fn pane_tabs_wrap_to_rows_that_fit() {
        let tab = |key, title| PaneTab {
            key,
            title,
            pane: 0,
        };
        let tabs = [
            tab('1', "Description"),
            tab('2', "Checks"),
            tab('3', "Conversation"),
        ];
        // With the edges, the tabs take 17, 12 and 18 cells. Two tabs on
        // one row have a gap of 1 cell.
        assert_eq!(pane_tab_rows(&tabs, 49), vec![0..3]);
        assert_eq!(pane_tab_rows(&tabs, 48), vec![0..2, 2..3]);
        assert_eq!(pane_tab_rows(&tabs, 30), vec![0..2, 2..3]);
        assert_eq!(pane_tab_rows(&tabs, 29), vec![0..1, 1..2, 2..3]);
        assert_eq!(pane_tab_rows(&tabs, 5), vec![0..1, 1..2, 2..3]);
        assert_eq!(pane_tab_rows(&[], 10), vec![0..0]);
    }

    #[test]
    fn a_second_row_of_pane_tabs_moves_the_pane_down() {
        let tabs: Vec<PaneTab> = [
            ('1', "Description"),
            ('2', "Checks"),
            ('3', "Conversation"),
            ('4', "Stack"),
            ('5', "Files"),
            ('6', "Agent"),
        ]
        .into_iter()
        .enumerate()
        .map(|(pane, (key, title))| PaneTab { key, title, pane })
        .collect();
        let pane = PaneModel::Lines {
            lines: vec![vec![crate::app::models::Cell {
                text: "first line".to_owned(),
                style: crate::app::models::CellStyle::Normal,
            }]],
            wrap: false,
        };
        let popup = Popup {
            title: "#7 title",
            subtitle: "subtitle",
            tabs: &tabs,
            active: 5,
            pane: &pane,
            cursor: 0,
            scroll: 0,
        };
        let area = Rect::new(0, 0, 60, 20);
        let mut buf = Buffer::empty(area);
        let mut hits = HitMap::default();
        render(&popup, area, &mut buf, &testdata::pierre(), &mut hits);
        let inner_width = 60 - 4;
        assert_eq!(pane_tab_rows(&tabs, inner_width).len(), 2);
        let first = row(&buf, 3);
        let second = row(&buf, 4);
        assert!(first.contains("1 Description") && first.contains("3 Conversation"));
        assert!(second.contains("4 Stack") && second.contains("6 Agent"));
        let agent = find(&buf, 4, "Agent");
        assert_eq!(hits.at(agent, 4), Some(Hit::PaneTab(5)));
        assert!(row(&buf, 5).contains('─'), "the rule is under the tabs");
        assert!(row(&buf, 6).contains("first line"));
    }

    /// A pane of `count` numbered lines.
    fn numbered_lines(count: usize) -> PaneModel {
        PaneModel::Lines {
            lines: (0..count)
                .map(|i| {
                    vec![crate::app::models::Cell {
                        text: format!("line {i}"),
                        style: crate::app::models::CellStyle::Normal,
                    }]
                })
                .collect(),
            wrap: true,
        }
    }

    /// Draws a popup of 40 x 16 cells with one pane tab, `pane` at
    /// `scroll`, in `theme`.
    fn draw_pane(pane: &PaneModel, scroll: usize, theme: &Theme) -> Buffer {
        let tabs = [PaneTab {
            key: '1',
            title: "Description",
            pane: 0,
        }];
        let popup = Popup {
            title: "#7 title",
            subtitle: "subtitle",
            tabs: &tabs,
            active: 0,
            pane,
            cursor: 0,
            scroll,
        };
        let area = Rect::new(0, 0, 40, 16);
        let mut buf = Buffer::empty(area);
        render(&popup, area, &mut buf, theme, &mut HitMap::default());
        buf
    }

    /// The rows of a buffer, joined with newlines.
    fn rows(buf: &Buffer) -> String {
        (0..buf.area.height)
            .map(|y| row(buf, y))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The column of the scrollbar: the right padding column, and the
    /// rows of the pane body.
    fn bar_cells(area: Rect) -> (u16, std::ops::Range<u16>) {
        let inner = ThemeBlock::inner(area);
        (inner.right() - 1, inner.y + HEADER_ROWS..inner.bottom())
    }

    #[test]
    fn popup_with_a_scrollbar() {
        insta::assert_snapshot!(rows(&draw_pane(
            &numbered_lines(30),
            10,
            &testdata::pierre()
        )));
    }

    #[test]
    fn popup_without_a_scrollbar() {
        insta::assert_snapshot!(rows(&draw_pane(&numbered_lines(3), 0, &testdata::pierre())));
    }

    #[test]
    fn the_scrollbar_shows_only_when_the_pane_scrolls() {
        let (x, body) = bar_cells(Rect::new(0, 0, 40, 16));
        let height = usize::from(body.end - body.start);
        for (count, shows) in [(height - 1, false), (height, false), (height + 1, true)] {
            let buf = draw_pane(&numbered_lines(count), 0, &testdata::pierre());
            let drawn = body.clone().any(|y| buf[(x, y)].symbol() != " ");
            assert_eq!(drawn, shows, "{count} lines");
        }
    }

    #[test]
    fn the_scrollbar_thumb_follows_the_scroll_in_each_theme() {
        use crate::theme::{PaletteChoice, ThemeKind, ThemeStyle};
        let plain = Theme::styled(
            ThemeKind::TrueColor,
            ThemeStyle::Plain,
            PaletteChoice::Pierre,
            true,
            true,
        );
        let themes = [
            ("grunge", testdata::pierre(), "┃"),
            ("plain", plain, "┃"),
            ("ansi16", testdata::ansi16(), "┃"),
            ("punk", testdata::punk(), "█"),
            ("punk ansi16", testdata::punk_ansi16(), "█"),
        ];
        let (x, body) = bar_cells(Rect::new(0, 0, 40, 16));
        let (top, bottom) = (body.start, body.end - 1);
        let pane = numbered_lines(40);
        for (name, theme, thumb) in themes {
            let p = theme.palette;
            let first = draw_pane(&pane, 0, &theme);
            assert_eq!(
                first[(x, top)].symbol(),
                thumb,
                "{name}: the thumb at the top"
            );
            assert_eq!(first[(x, top)].fg, p.primary, "{name}");
            assert_eq!(first[(x, bottom)].symbol(), "│", "{name}: the track");
            assert_eq!(first[(x, bottom)].fg, p.surface, "{name}");
            assert_eq!(first[(x, bottom)].bg, p.bg, "{name}");
            let last = draw_pane(&pane, 99, &theme);
            assert_eq!(last[(x, top)].symbol(), "│", "{name}: the track");
            assert_eq!(
                last[(x, bottom)].symbol(),
                thumb,
                "{name}: the thumb at the end"
            );
        }
    }

    #[test]
    fn a_table_scrollbar_starts_below_the_header_row() {
        let checks = testdata::checks_pane();
        let PaneModel::Table { columns, rows } = checks else {
            unreachable!()
        };
        let many = (0..20).flat_map(|_| rows.clone()).collect();
        let pane = PaneModel::Table {
            columns,
            rows: many,
        };
        let buf = draw_pane(&pane, 0, &testdata::pierre());
        let (x, body) = bar_cells(Rect::new(0, 0, 40, 16));
        assert_eq!(buf[(x, body.start)].symbol(), " ", "the header row");
        assert_eq!(buf[(x, body.start + 1)].symbol(), "┃");
    }

    #[test]
    fn hits_of_the_sidebar() {
        let areas = testdata::areas(100, 30);
        let area = crate::ui::rect(areas.popup);
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
        let mut hits = HitMap::default();
        let crate::app::models::OverlayContent::Popup {
            title,
            subtitle,
            tabs,
            active,
            pane,
            cursor,
            scroll,
        } = testdata::popup_overlay().content
        else {
            unreachable!()
        };
        let popup = Popup {
            title: &title,
            subtitle: &subtitle,
            tabs: &tabs,
            active,
            pane: &pane,
            cursor,
            scroll,
        };
        render(&popup, area, &mut buf, &testdata::pierre(), &mut hits);
        assert_eq!(hits.at(area.x - 1, 10), None, "left of the sidebar");
        assert_eq!(hits.at(area.x, 10), Some(Hit::Inside), "the border");
        assert_eq!(
            hits.at(area.x + 2, area.y + 1),
            Some(Hit::Inside),
            "the title"
        );
        let tabs_y = area.y + 3;
        let checks = find(&buf, tabs_y, "Checks");
        assert_eq!(hits.at(checks, tabs_y), Some(Hit::PaneTab(1)));
        let stack = find(&buf, tabs_y, "Stack");
        assert_eq!(hits.at(stack, tabs_y), Some(Hit::PaneTab(3)));
        let test_y = (0..30)
            .find(|&y| row(&buf, y).contains("ci / test"))
            .unwrap();
        assert_eq!(hits.at(area.x + 5, test_y), Some(Hit::PaneRow(1)));
        assert_eq!(hits.at(area.x + 5, test_y - 1), Some(Hit::PaneRow(0)));
        assert_eq!(
            hits.at(area.x + 5, test_y - 2),
            Some(Hit::Inside),
            "the header row"
        );
    }
}
