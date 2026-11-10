//! The Send review dialog: the comments of the review, the first lines of
//! the message, one row per button, and the cancel keys. A blocked button
//! shows dim with its reason. The button that `enter` presses shows the
//! cursor (`>`).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Widget};

use crate::bus::{Hit, SendReviewSpec};
use crate::theme::Theme;
use crate::theme::ThemeBlock;
use crate::ui::hits::HitMap;
use crate::ui::text;

/// The text of an empty message.
const NO_MESSAGE: &str = "(no message, press m to write one)";

/// The width of the label column of the buttons.
const LABEL_WIDTH: usize = 18;

/// The height of the dialog: the frame, the comment lines, a blank row,
/// the message title and its lines (at least one), a blank row, the
/// buttons, a blank row and the cancel row.
pub(crate) fn height(spec: &SendReviewSpec) -> u16 {
    let rows = spec.lines.len() + spec.message.len().max(1) + spec.buttons.len() + 5;
    u16::try_from(rows + 2).unwrap_or(u16::MAX)
}

/// Draws the dialog into its area. The whole area is a `Hit::Inside`.
/// When the area is too low, the comment lines lose rows first.
pub(crate) fn render(
    spec: &SendReviewSpec,
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
    let width = usize::from(body.width);
    let normal = Style::new().fg(p.fg);
    let mute = Style::new().fg(p.fg_mute);
    let key = Style::new().fg(p.primary).add_modifier(Modifier::BOLD);
    let bold = Style::new().fg(p.fg).add_modifier(Modifier::BOLD);

    let mut rest: Vec<Line<'_>> = vec![Line::default(), Line::styled("Message", key)];
    if spec.message.is_empty() {
        rest.push(Line::styled(
            NO_MESSAGE,
            mute.add_modifier(Modifier::ITALIC),
        ));
    }
    for line in &spec.message {
        rest.push(Line::styled(text::truncate(line, width), normal));
    }
    rest.push(Line::default());
    for button in &spec.buttons {
        let pointer = if button.ready { "> " } else { "  " };
        let label = format!("  {:<LABEL_WIDTH$}", button.label);
        let mut spans = vec![Span::styled(pointer, key)];
        if let Some(reason) = button.blocked {
            spans.push(Span::styled(button.key.to_string(), mute));
            spans.push(Span::styled(label, mute));
            spans.push(Span::styled(reason, mute.add_modifier(Modifier::ITALIC)));
        } else {
            let style = if button.ready { bold } else { normal };
            spans.push(Span::styled(button.key.to_string(), key));
            spans.push(Span::styled(label, style));
        }
        if button.ready {
            spans.push(Span::styled(" · enter", mute));
        }
        rest.push(Line::from(spans));
    }
    rest.push(Line::default());
    rest.push(Line::from(vec![
        Span::styled("n", bold),
        Span::styled(" / ", mute),
        Span::styled("Esc", bold),
        Span::styled(" cancel", mute),
    ]));

    let rows = usize::from(body.height);
    let keep = rows.saturating_sub(rest.len());
    let comments = spec
        .lines
        .iter()
        .take(keep)
        .map(|line| Line::styled(text::truncate(line, width), normal));
    for (y, line) in (body.y..body.bottom()).zip(comments.chain(rest)) {
        line.render(Rect::new(body.x, y, body.width, 1), buf);
    }
}
