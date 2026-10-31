//! The popup frame with pane tabs: the header with the title and the
//! subtitle, the pane tabs, and the active pane below. The transitions
//! slide it in from the bottom and out again.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};

use crate::app::models::{PaneModel, PaneTab};
use crate::theme::Theme;
use crate::theme::ThemeBlock;
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

/// The pane tabs: `1 Checks  2 Conversation  3 Stack`, with the active
/// tab marked.
fn tabs_line(tabs: &[PaneTab], active: usize, theme: &Theme) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, tab) in tabs.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        let label = format!(" {} {} ", tab.key, tab.title);
        spans.extend(theme.tab(&label, i == active));
    }
    Line::from(spans)
}

/// Draws the popup into its area.
pub(crate) fn render(popup: &Popup<'_>, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let p = &theme.palette;
    let area = area.intersection(buf.area);
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
    tabs_line(popup.tabs, popup.active, theme)
        .render(Rect::new(inner.x, inner.y + 2, inner.width, 1), buf);
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
    pane::render(popup.pane, popup.cursor, popup.scroll, body, buf, theme);
}
