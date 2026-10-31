//! The popup frame with pane tabs: the header with the title and the
//! subtitle, the pane tabs, and the active pane below. The transitions
//! slide it in from the right and out again.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};

use crate::app::models::{PaneModel, PaneTab};
use crate::bus::Hit;
use crate::theme::Theme;
use crate::theme::ThemeBlock;
use crate::ui::hits::HitMap;
use crate::ui::{pane, text};

/// The rows above the pane: title, subtitle, pane tabs and a rule.
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

/// The space between two pane tabs.
const TAB_GAP: &str = "  ";

/// The pane tabs: `1 Description  2 Checks  3 Conversation  4 Stack`, with
/// the active tab marked. Each tab label in `area` is a `Hit::PaneTab`
/// with its index in `tabs`.
fn tabs_line(
    tabs: &[PaneTab],
    active: usize,
    theme: &Theme,
    area: Rect,
    hits: &mut HitMap,
) -> Line<'static> {
    let mut spans = Vec::new();
    let mut x = area.x;
    for (i, tab) in tabs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(TAB_GAP));
            x = x.saturating_add(u16::try_from(TAB_GAP.len()).unwrap_or(u16::MAX));
        }
        let label = format!(" {} {} ", tab.key, tab.title);
        let tab = theme.tab(&label, i == active);
        let width = tab.iter().map(Span::width).sum::<usize>();
        let width = u16::try_from(width).unwrap_or(u16::MAX);
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
    if inner.height < HEADER_ROWS || inner.width == 0 {
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
    let tabs_area = Rect::new(inner.x, inner.y + 2, inner.width, 1);
    tabs_line(popup.tabs, popup.active, theme, tabs_area, hits).render(tabs_area, buf);
    buf.set_span(
        inner.x,
        inner.y + 3,
        &theme.separator(width, Style::new().fg(p.fg_mute)),
        inner.width,
    );
    let body = Rect::new(
        inner.x,
        inner.y + HEADER_ROWS,
        inner.width,
        inner.height - HEADER_ROWS,
    );
    pane::render(
        popup.pane,
        popup.cursor,
        popup.scroll,
        body,
        buf,
        theme,
        hits,
    );
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
