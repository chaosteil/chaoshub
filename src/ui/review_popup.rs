//! The Review popup: the list of the codeview comments on top, then a
//! blank row, then two columns. The left column stacks the keys, and the
//! right column shows the message, wrapped to its width. The list scrolls
//! so the cursor stays visible, with a scrollbar when it scrolls. A
//! blocked key shows dim with its reason.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, ScrollbarState, StatefulWidget, Widget};

use crate::app::measure::wrap_ranges;
use crate::app::models::{REVIEW_FRAME_ROWS, REVIEW_LIST_ROWS};
use crate::app::nav::keep_visible;
use crate::bus::{Hit, ReviewRow, ReviewSpec};
use crate::theme::Theme;
use crate::theme::ThemeBlock;
use crate::ui::hits::HitMap;
use crate::ui::text;

/// The rows under the comment list: a blank row and the five key rows.
const FOOT_ROWS: usize = REVIEW_FRAME_ROWS - 2;

/// The width of the key column of a key row.
const KEY_WIDTH: usize = 4;

/// The width of the label column of a key row.
const LABEL_WIDTH: usize = 17;

/// The width of the left column: the key, the label and the longest
/// reason.
const KEYS_WIDTH: u16 = 36;

/// The blank columns between the keys and the message.
const GAP: u16 = 2;

/// The most rows of the message. A last row counts the rest.
const MESSAGE_ROWS: usize = 4;

/// The height of the popup: the frame, the visible rows of the comment
/// list, a blank row and the five key rows.
pub(crate) fn height(spec: &ReviewSpec) -> u16 {
    let rows = spec.rows.len().clamp(1, REVIEW_LIST_ROWS) + REVIEW_FRAME_ROWS;
    u16::try_from(rows).unwrap_or(u16::MAX)
}

/// Draws the popup into its area. The whole area is a `Hit::Inside`, and
/// each drawn row of the comment list is a `Hit::Item` with its index.
/// When the area is too low, the comment list loses rows first.
pub(crate) fn render(
    spec: &ReviewSpec,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let p = &theme.palette;
    let area = area.intersection(buf.area);
    hits.push(area, Hit::Inside);
    Clear.render(area, buf);
    theme
        .block(&spec.title)
        .accent(p.primary)
        .seed(23)
        .render(area, buf);
    let inner = ThemeBlock::inner(area);
    if inner.height < 1 || inner.width < 2 {
        return;
    }
    let body = Rect::new(inner.x + 1, inner.y, inner.width - 2, inner.height);
    let rows = usize::from(body.height);
    let list = rows.saturating_sub(FOOT_ROWS).min(spec.rows.len());
    let list_area = Rect::new(body.x, body.y, body.width, u16::try_from(list).unwrap_or(0));
    render_list(spec, list_area, buf, theme, hits);
    if list < spec.rows.len() && list > 0 {
        let scroll = keep_visible(spec.rows.len(), spec.cursor.unwrap_or(0), spec.scroll, list);
        let bar = Rect::new(inner.right() - 1, list_area.y, 1, list_area.height);
        let mut state = ScrollbarState::new(spec.rows.len() - list + 1)
            .position(scroll)
            .viewport_content_length(list);
        theme.scrollbar().render(bar, buf, &mut state);
    }
    let foot_y = list_area.bottom() + 1;
    if foot_y >= body.bottom() {
        return;
    }
    let foot = Rect::new(body.x, foot_y, body.width, body.bottom() - foot_y);
    let keys = Rect::new(foot.x, foot.y, KEYS_WIDTH.min(foot.width), foot.height);
    for (y, line) in (keys.y..keys.bottom()).zip(key_lines(spec, theme)) {
        line.render(Rect::new(keys.x, y, keys.width, 1), buf);
    }
    let left = KEYS_WIDTH + GAP;
    if foot.width > left {
        let width = foot.width - left;
        let lines = message_lines(&spec.message, usize::from(width), theme);
        for (y, line) in (foot.y..foot.bottom()).zip(lines) {
            line.render(Rect::new(foot.x + left, y, width, 1), buf);
        }
    }
}

/// Draws the visible rows of the comment list into `area`.
fn render_list(spec: &ReviewSpec, area: Rect, buf: &mut Buffer, theme: &Theme, hits: &mut HitMap) {
    let p = &theme.palette;
    let height = usize::from(area.height);
    if height == 0 {
        return;
    }
    let width = usize::from(area.width);
    let scroll = keep_visible(
        spec.rows.len(),
        spec.cursor.unwrap_or(0),
        spec.scroll,
        height,
    );
    let normal = Style::new().fg(p.fg);
    let mute = Style::new().fg(p.fg_mute);
    let key = Style::new().fg(p.primary).add_modifier(Modifier::BOLD);
    let rows = spec.rows.iter().enumerate().skip(scroll).take(height);
    for (y, (index, row)) in (area.y..).zip(rows) {
        let line_area = Rect::new(area.x, y, area.width, 1);
        hits.push(line_area, Hit::Item(index));
        let cursor = spec.cursor == Some(index);
        let pointer = if cursor { "> " } else { "  " };
        let line = match row {
            ReviewRow::Comment {
                marked,
                place,
                summary,
            } => {
                let mark = if *marked { "[x] " } else { "[ ] " };
                let lead = format!("{pointer}{mark}");
                let place = format!("{place}  ");
                let room = width.saturating_sub(text::width(&lead));
                let rest = room.saturating_sub(text::width(&place));
                let style = if cursor {
                    normal.add_modifier(Modifier::BOLD)
                } else {
                    normal
                };
                Line::from(vec![
                    Span::styled(lead, key),
                    Span::styled(text::truncate(&place, room), Style::new().fg(p.info)),
                    Span::styled(text::truncate(summary, rest), style),
                ])
            }
            ReviewRow::Fixed { place, summary } => Line::styled(
                text::truncate(&format!("  [-] {place}  {summary}"), width),
                mute,
            ),
            ReviewRow::Note(note) => Line::styled(text::truncate(note, width), mute),
            ReviewRow::Error(err) => {
                Line::styled(text::truncate(err, width), Style::new().fg(p.danger))
            }
        };
        line.render(line_area, buf);
    }
}

/// The key rows: one row per button, then the cancel row.
fn key_lines<'a>(spec: &'a ReviewSpec, theme: &Theme) -> Vec<Line<'a>> {
    let p = &theme.palette;
    let normal = Style::new().fg(p.fg);
    let mute = Style::new().fg(p.fg_mute);
    let key = Style::new().fg(p.primary).add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line<'_>> = spec
        .buttons
        .iter()
        .map(|button| {
            let name = format!("{:<KEY_WIDTH$}", button.key);
            match button.blocked {
                Some(reason) => Line::from(vec![
                    Span::styled(name, mute),
                    Span::styled(format!("{:<LABEL_WIDTH$}", button.label), mute),
                    Span::styled(reason, mute.add_modifier(Modifier::ITALIC)),
                ]),
                None => Line::from(vec![
                    Span::styled(name, key),
                    Span::styled(button.label, normal),
                ]),
            }
        })
        .collect();
    lines.push(Line::from(vec![
        Span::styled(format!("{:<KEY_WIDTH$}", "Esc"), key),
        Span::styled("cancel", normal),
    ]));
    lines
}

/// The rows of the message, wrapped to `width`: at most `MESSAGE_ROWS`
/// rows, then a row that counts the rest. Empty without a message.
fn message_lines<'a>(message: &'a str, width: usize, theme: &Theme) -> Vec<Line<'a>> {
    let p = &theme.palette;
    if message.trim().is_empty() {
        return Vec::new();
    }
    let rows: Vec<&str> = message
        .lines()
        .flat_map(|line| {
            wrap_ranges(line, width)
                .into_iter()
                .map(|range| &line[range])
        })
        .collect();
    let mut lines: Vec<Line<'_>> = rows
        .iter()
        .take(MESSAGE_ROWS)
        .map(|&row| Line::styled(row, Style::new().fg(p.fg)))
        .collect();
    if rows.len() > MESSAGE_ROWS {
        let more = rows.len() - MESSAGE_ROWS;
        let noun = if more == 1 { "line" } else { "lines" };
        lines.push(Line::styled(
            format!("… {more} more {noun}"),
            Style::new().fg(p.fg_mute),
        ));
    }
    lines
}
