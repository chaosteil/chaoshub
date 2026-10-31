//! The tab bar: the tab titles in registry order and the active tab.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::Widget;

use crate::app::models::TabBarModel;
use crate::theme::Theme;

/// The name on the right end of the bar.
const NAME: &str = "chaoshub";

/// Draws the tab bar. The active tab has the `▌` marker and the surface
/// background. The free cells get the texture.
pub(crate) fn render(model: &TabBarModel, area: Rect, buf: &mut Buffer, theme: &Theme) {
    let p = &theme.palette;
    let base = Style::new().bg(p.bg).fg(p.fg_mute);
    buf.set_style(area, base);
    let mut spans = Vec::new();
    for (i, title) in model.titles.iter().enumerate() {
        let label = format!(" {} {} ", i + 1, theme.tab_title(title));
        spans.extend(theme.tab(&label, i == model.active));
    }
    let line = Line::from(spans);
    let used = u16::try_from(line.width())
        .unwrap_or(u16::MAX)
        .min(area.width);
    line.render(area, buf);
    let name_width = u16::try_from(NAME.len()).unwrap_or(0) + 1;
    let free = Rect::new(
        area.x + used,
        area.y,
        area.width.saturating_sub(used),
        area.height,
    );
    if free.width > name_width + 1 {
        let x = area.right() - name_width;
        buf.set_string(x, area.y, NAME, Style::new().bg(p.bg).fg(p.fg_mute));
        let texture = Rect::new(free.x + 1, free.y, free.width - name_width - 2, 1);
        theme.fill(texture, buf, 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::testdata;

    fn row(buf: &Buffer) -> String {
        (0..buf.area.width).map(|x| buf[(x, 0)].symbol()).collect()
    }

    fn draw(width: u16, active: usize, theme: &Theme) -> Buffer {
        let area = Rect::new(0, 0, width, 1);
        let mut buf = Buffer::empty(area);
        let model = TabBarModel {
            titles: vec!["mine", "team"],
            active,
        };
        render(&model, area, &mut buf, theme);
        buf
    }

    #[test]
    fn tab_bar_100_pierre() {
        insta::assert_snapshot!(row(&draw(100, 0, &testdata::pierre())));
    }

    #[test]
    fn tab_bar_160_ansi16() {
        insta::assert_snapshot!(row(&draw(160, 1, &testdata::ansi16())));
    }

    #[test]
    fn active_tab_is_marked() {
        let theme = testdata::pierre();
        let buf = draw(100, 1, &theme);
        let text = row(&buf);
        let marker = text.chars().position(|c| c == '▌').unwrap();
        assert!(
            text.chars()
                .skip(marker)
                .collect::<String>()
                .starts_with("▌ 2 T E A M")
        );
        assert_eq!(
            buf[(u16::try_from(marker).unwrap(), 0)].fg,
            theme.palette.primary
        );
    }

    #[test]
    fn punk_active_tab_is_angled_and_red() {
        let theme = testdata::punk();
        let buf = draw(100, 0, &theme);
        let text = row(&buf);
        assert!(text.starts_with("◤ 1 MINE ◢  2 TEAM  "), "{text}");
        assert_eq!(buf[(2, 0)].bg, theme.palette.primary);
        assert_eq!(buf[(2, 0)].fg, theme.palette.bg);
        assert_eq!(buf[(12, 0)].bg, theme.palette.bg);
    }
}
