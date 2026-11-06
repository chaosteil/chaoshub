//! The tab bar: the tab titles in registry order and the active tab.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::app::models::TabBarModel;
use crate::bus::Hit;
use crate::theme::Theme;
use crate::ui::hits::HitMap;

/// The name on the right end of the bar.
const NAME: &str = "chaoshub";

/// Draws the tab bar. The active tab has the `▌` marker and the surface
/// background. The free cells get the texture. Each tab label is a
/// `Hit::Tab`. An `active` index past the last tab draws the bar with no
/// active tab: every tab has the inactive style.
pub(crate) fn render(
    model: &TabBarModel,
    area: Rect,
    buf: &mut Buffer,
    theme: &Theme,
    hits: &mut HitMap,
) {
    let p = &theme.palette;
    let base = Style::new().bg(p.bg).fg(p.fg_mute);
    buf.set_style(area, base);
    let mut spans = Vec::new();
    let mut x = area.x;
    let labels: Vec<String> = model
        .titles
        .iter()
        .enumerate()
        .map(|(i, title)| format!(" {} {} ", i + 1, theme.tab_title(title)))
        .collect();
    for (i, tab) in theme
        .tab_strip(&labels, model.active)
        .into_iter()
        .enumerate()
    {
        let width = tab.iter().map(Span::width).sum::<usize>();
        let width = u16::try_from(width).unwrap_or(u16::MAX);
        let label_area = Rect::new(x, area.y, width, area.height.min(1)).intersection(area);
        hits.push(label_area, Hit::Tab(i));
        x = x.saturating_add(width);
        spans.extend(tab);
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
        render(&model, area, &mut buf, theme, &mut HitMap::default());
        buf
    }

    #[test]
    fn each_tab_label_is_a_hit() {
        let area = Rect::new(0, 0, 100, 1);
        let mut buf = Buffer::empty(area);
        let model = TabBarModel {
            titles: vec!["mine", "team"],
            active: 0,
        };
        let mut hits = HitMap::default();
        render(&model, area, &mut buf, &testdata::punk(), &mut hits);
        // `◢ 1 MINE ◤ 2 TEAM ◤`: the first tab is 10 cells wide, the
        // second 9.
        assert_eq!(hits.at(0, 0), Some(Hit::Tab(0)));
        assert_eq!(hits.at(9, 0), Some(Hit::Tab(0)));
        assert_eq!(hits.at(10, 0), Some(Hit::Tab(1)));
        assert_eq!(hits.at(18, 0), Some(Hit::Tab(1)));
        assert_eq!(hits.at(19, 0), None);
        assert_eq!(hits.at(30, 0), None, "the free cells hit nothing");
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
    fn punk_active_tab_is_a_blue_card() {
        let theme = testdata::punk();
        let p = theme.palette;
        let buf = draw(100, 0, &theme);
        let text = row(&buf);
        assert!(text.starts_with("◢ 1 MINE ◤ 2 TEAM ◤ "), "{text}");
        assert_eq!(buf[(0, 0)].fg, p.primary);
        assert_eq!(buf[(0, 0)].bg, p.bg);
        assert_eq!(buf[(2, 0)].bg, p.primary);
        assert_eq!(buf[(2, 0)].fg, p.bg);
        // The blue card cuts into the grey card in one shared cell.
        assert_eq!(buf[(9, 0)].fg, p.primary);
        assert_eq!(buf[(9, 0)].bg, p.fg_mute);
        // The other tab is a grey card with black text.
        assert_eq!(buf[(11, 0)].bg, p.fg_mute);
        assert_eq!(buf[(11, 0)].fg, p.bg);
        assert_eq!(buf[(18, 0)].fg, p.fg_mute);
        assert_eq!(buf[(18, 0)].bg, p.bg);
        // An inactive tab before the active one ends grey on blue.
        let buf = draw(100, 1, &theme);
        assert_eq!(buf[(9, 0)].fg, p.fg_mute);
        assert_eq!(buf[(9, 0)].bg, p.primary);
        assert_eq!(buf[(11, 0)].bg, p.primary);
    }
}
