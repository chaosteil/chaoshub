//! The theme: palette, skin, borders and textures.
#![allow(dead_code, reason = "ui and run use the theme in later phases")]

mod block;
pub(crate) mod grunge;
mod palette;
pub(crate) mod punk;
pub(crate) mod texture;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, Widget};

pub(crate) use block::ThemeBlock;
use grunge::stencil;
use texture::Texture;

/// The color support of the terminal, which `main` detects from
/// `COLORTERM` and `TERM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThemeKind {
    TrueColor,
    Ansi16,
}

/// The colors of the theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Palette {
    pub bg: Color,
    pub surface: Color,
    pub fg: Color,
    pub fg_dim: Color,
    pub fg_mute: Color,
    pub primary: Color,
    pub success: Color,
    pub danger: Color,
    pub warn: Color,
    pub info: Color,
    pub merge: Color,
    pub magenta: Color,
    pub orange: Color,
    pub purple: Color,
    pub pink: Color,
}

/// The look of the frames, headers, separators, fills, selections and
/// tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Skin {
    /// Rough block borders, stencil headers, and textures in empty areas
    /// when `textures` is true.
    Grunge { textures: bool },
    /// Angular cut corners, diagonal slashes and inverted headers.
    Punk,
    /// Rounded borders, stencil headers and no textures.
    Plain,
}

/// The theme style that the user picks with `[theme] style` or
/// `--theme`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ThemeStyle {
    Grunge,
    Punk,
    Plain,
}

/// The palette that the user picks with `[theme] palette` or
/// `--palette`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PaletteChoice {
    Pierre,
    Ansi16,
}

/// The palette and the skin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Theme {
    pub palette: Palette,
    pub skin: Skin,
}

/// How the theme draws the status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StatusBand {
    /// The style of the whole bar.
    pub bar: Style,
    /// The style of the left part, over the cell styles. The band starts
    /// at the left edge of the bar. `None` keeps the cell styles.
    pub left: Option<Style>,
    /// The glyph after the left part, one band cell after its text, and
    /// its style.
    pub end: Option<(&'static str, Style)>,
    /// The text color of a message without a state. `None` keeps the
    /// cell color.
    pub message_fg: Option<Color>,
}

/// How the theme draws the selected row of a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Selection {
    /// The style to patch over the base style of the selected row.
    pub patch: Style,
    /// The glyph in the cursor gutter.
    pub marker: &'static str,
    /// The color of the marker.
    pub marker_fg: Color,
    /// The background of the marker cell. `None` keeps the band
    /// background.
    pub marker_bg: Option<Color>,
    /// The two glyphs at the right end of the row, and their styles. A
    /// row that has a tail keeps its last two cells free for it.
    pub tail: Option<[(&'static str, Style); 2]>,
    /// The text color of every cell on the selected row. `None` keeps the
    /// colors of the cells.
    pub cell_fg: Option<Color>,
    /// The cells that the text of the selected row moves to the right.
    /// Every row keeps this many cells free at its right end.
    pub shift: u16,
}

impl Theme {
    /// Builds the theme for a terminal kind. `grunge` and `textures` come
    /// from the configuration. Without `grunge`, the skin is plain. A
    /// 16-color terminal gets the ANSI palette and the plain skin.
    pub(crate) const fn new(kind: ThemeKind, grunge: bool, textures: bool) -> Self {
        Self::styled(
            kind,
            ThemeStyle::Grunge,
            PaletteChoice::Pierre,
            grunge,
            textures,
        )
    }

    /// Builds the theme of a style and a palette for a terminal kind. The
    /// Pierre palette needs a true-color terminal; else the theme uses the
    /// ANSI palette. `grunge` and `textures` apply to the grunge style
    /// only. The grunge style with the ANSI palette gets the plain skin.
    pub(crate) const fn styled(
        kind: ThemeKind,
        style: ThemeStyle,
        palette: PaletteChoice,
        grunge: bool,
        textures: bool,
    ) -> Self {
        let (palette, true_color) = match (palette, kind) {
            (PaletteChoice::Pierre, ThemeKind::TrueColor) => (Palette::pierre(), true),
            _ => (Palette::ansi16(), false),
        };
        let skin = match style {
            ThemeStyle::Grunge if grunge && true_color => Skin::Grunge { textures },
            ThemeStyle::Punk => Skin::Punk,
            ThemeStyle::Grunge | ThemeStyle::Plain => Skin::Plain,
        };
        Self { palette, skin }
    }

    /// Builds the punk theme for a terminal kind: the Pierre palette, or
    /// the 16 ANSI colors, with the punk skin.
    pub(crate) const fn punk(kind: ThemeKind) -> Self {
        Self::styled(kind, ThemeStyle::Punk, PaletteChoice::Pierre, true, true)
    }

    /// A framed block with a header on the top edge. An empty `title`
    /// gives a block without a header.
    pub(crate) fn block<'a>(&self, title: &'a str) -> ThemeBlock<'a> {
        ThemeBlock::new(self, title)
    }

    /// A header line in the color `color`. Grunge and plain headers are
    /// stencils in the color. Punk headers are inverted: black text on a
    /// block of the color.
    pub(crate) fn header(&self, text: &str, color: Color) -> Line<'static> {
        match self.skin {
            Skin::Punk => punk::inverted(text, color, self.palette.bg),
            Skin::Grunge { .. } | Skin::Plain => stencil(text, Style::new().fg(color)),
        }
    }

    /// A horizontal rule of `width` cells.
    pub(crate) fn separator(&self, width: usize, style: Style) -> Span<'static> {
        match self.skin {
            Skin::Punk => Span::styled(punk::separator(width), style),
            Skin::Grunge { .. } | Skin::Plain => Span::styled("─".repeat(width), style),
        }
    }

    /// The scrollbar of a pane: a thin track in the surface color and a
    /// thumb in the primary color, the accent of the popup border. Punk
    /// draws a solid thumb, like its cards. The track does not show on a
    /// 16-color terminal, where the surface is the background.
    pub(crate) fn scrollbar(&self) -> Scrollbar<'static> {
        let p = &self.palette;
        let thumb = match self.skin {
            Skin::Punk => "█",
            Skin::Grunge { .. } | Skin::Plain => "┃",
        };
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some("│"))
            .track_style(Style::new().fg(p.surface).bg(p.bg))
            .thumb_symbol(thumb)
            .thumb_style(Style::new().fg(p.primary).bg(p.bg))
    }

    /// Fills the empty cells of an area. `seed` picks the pattern. Only
    /// the grunge skin with textures draws a fill.
    pub(crate) fn fill(&self, area: Rect, buf: &mut Buffer, seed: u64) {
        Texture::new(self).seed(seed).render(area, buf);
    }

    /// How the selected row of a list looks. Grunge and plain rows get
    /// the surface background and a `▌` marker. A punk row is a blue card
    /// on a white card: `▶` in black on white, the cells in black on the
    /// primary color, a `◤◤` tail, and the text one cell to the right.
    pub(crate) fn selected_row(&self) -> Selection {
        let p = &self.palette;
        match self.skin {
            Skin::Punk => Selection {
                patch: Style::new().bg(p.primary).fg(p.bg),
                marker: "▶",
                marker_fg: p.bg,
                marker_bg: Some(p.fg),
                tail: Some([
                    ("◤", Style::new().bg(p.fg).fg(p.primary)),
                    ("◤", Style::new().bg(p.bg).fg(p.fg)),
                ]),
                cell_fg: Some(p.bg),
                shift: 1,
            },
            Skin::Grunge { .. } | Skin::Plain => Selection {
                patch: Style::new().bg(p.surface),
                marker: "▌",
                marker_fg: p.primary,
                marker_bg: None,
                tail: None,
                cell_fg: None,
                shift: 0,
            },
        }
    }

    /// The title of a tab in the tab bar: uppercase, and letter-spaced
    /// for grunge and plain.
    pub(crate) fn tab_title(&self, title: &str) -> String {
        let upper = title.to_uppercase();
        match self.skin {
            Skin::Punk => upper,
            Skin::Grunge { .. } | Skin::Plain => upper
                .chars()
                .map(String::from)
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    /// The spans of one tab with its label. The active tab stands out.
    /// A punk tab is a slanted card, `◢ label ◤`: the active one has
    /// black bold text on the primary color, the others have black text
    /// on grey.
    pub(crate) fn tab(&self, label: &str, active: bool) -> Vec<Span<'static>> {
        let p = &self.palette;
        if self.skin == Skin::Punk {
            let (card, text) = if active {
                (p.primary, Style::new().add_modifier(Modifier::BOLD))
            } else {
                (p.fg_mute, Style::new())
            };
            let edge = Style::new().bg(p.bg).fg(card);
            return vec![
                Span::styled("◢", edge),
                Span::styled(label.to_owned(), text.bg(card).fg(p.bg)),
                Span::styled("◤", edge),
            ];
        }
        if active {
            let style = Style::new().bg(p.surface).fg(p.fg);
            vec![
                Span::styled("▌", style.fg(p.primary)),
                Span::styled(label.to_owned(), style.add_modifier(Modifier::BOLD)),
            ]
        } else {
            let style = Style::new().bg(p.bg).fg(p.fg_mute);
            vec![
                Span::styled(" ", style),
                Span::styled(label.to_owned(), style),
            ]
        }
    }

    /// The background and the foreground of the active tab: black on the
    /// primary color for punk, the surface background for grunge and
    /// plain.
    pub(crate) fn active_tab_colors(&self) -> (Color, Color) {
        let p = &self.palette;
        match self.skin {
            Skin::Punk => (p.primary, p.bg),
            Skin::Grunge { .. } | Skin::Plain => (p.surface, p.fg),
        }
    }

    /// The spans of each tab of a tab bar, in order. Grunge and plain tabs
    /// are the same as `tab`. Punk tabs are joined cards: the first tab
    /// starts with `◢`, and each tab ends with one `◤` in its own card
    /// color on the card color of the next tab, so two tabs share one
    /// edge cell.
    pub(crate) fn tab_strip(&self, labels: &[String], active: usize) -> Vec<Vec<Span<'static>>> {
        let p = &self.palette;
        if self.skin != Skin::Punk {
            return labels
                .iter()
                .enumerate()
                .map(|(i, label)| self.tab(label, i == active))
                .collect();
        }
        let card = |i: usize| if i == active { p.primary } else { p.fg_mute };
        labels
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let text = if i == active {
                    Style::new().add_modifier(Modifier::BOLD)
                } else {
                    Style::new()
                };
                let next = if i + 1 < labels.len() {
                    card(i + 1)
                } else {
                    p.bg
                };
                let mut spans = Vec::new();
                if i == 0 {
                    spans.push(Span::styled("◢", Style::new().bg(p.bg).fg(card(i))));
                }
                spans.push(Span::styled(label.clone(), text.bg(card(i)).fg(p.bg)));
                spans.push(Span::styled("◤", Style::new().bg(next).fg(card(i))));
                spans
            })
            .collect()
    }

    /// How the status bar looks. Grunge and plain bars have the surface
    /// background. The punk bar has a band in the primary color with
    /// black text on the left, a diagonal end, and the message in white.
    pub(crate) fn status_band(&self) -> StatusBand {
        let p = &self.palette;
        match self.skin {
            Skin::Punk => StatusBand {
                bar: Style::new().bg(p.bg).fg(p.fg),
                left: Some(
                    Style::new()
                        .bg(p.primary)
                        .fg(p.bg)
                        .add_modifier(Modifier::BOLD),
                ),
                end: Some(("◤", Style::new().bg(p.bg).fg(p.primary))),
                message_fg: Some(p.fg),
            },
            Skin::Grunge { .. } | Skin::Plain => StatusBand {
                bar: Style::new().bg(p.surface).fg(p.fg),
                left: None,
                end: None,
                message_fg: None,
            },
        }
    }

    /// The marker before the first key hint, in place of `prefix`, with
    /// the same width.
    pub(crate) fn hint_prefix(&self, prefix: &'static str) -> Span<'static> {
        let p = &self.palette;
        let style = Style::new().bg(p.bg).fg(p.primary);
        match self.skin {
            Skin::Punk => {
                let width = prefix.chars().count();
                let mut text = "╱".repeat(width.saturating_sub(1));
                if width > 0 {
                    text.push(' ');
                }
                Span::styled(text, style.add_modifier(Modifier::BOLD))
            }
            Skin::Grunge { .. } | Skin::Plain => Span::styled(prefix, style),
        }
    }

    /// True when the skin draws textures in empty areas.
    pub(crate) const fn textures(&self) -> bool {
        matches!(self.skin, Skin::Grunge { textures: true })
    }

    /// True when the skin draws the rough grunge frames.
    pub(crate) const fn grunge(&self) -> bool {
        matches!(self.skin, Skin::Grunge { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn true_color_keeps_the_switches() {
        let theme = Theme::new(ThemeKind::TrueColor, true, false);
        assert_eq!(theme.palette, Palette::pierre());
        assert_eq!(theme.skin, Skin::Grunge { textures: false });
        assert!(!theme.textures());
    }

    #[test]
    fn no_grunge_is_the_plain_skin() {
        let theme = Theme::new(ThemeKind::TrueColor, false, true);
        assert_eq!(theme.skin, Skin::Plain);
        assert!(!theme.textures());
    }

    #[test]
    fn ansi16_turns_grunge_and_textures_off() {
        let theme = Theme::new(ThemeKind::Ansi16, true, true);
        assert_eq!(theme.palette, Palette::ansi16());
        assert_eq!(theme.skin, Skin::Plain);
        assert!(!theme.grunge());
        assert!(!theme.textures());
    }

    #[test]
    fn styled_picks_the_skin() {
        let kind = ThemeKind::TrueColor;
        let pierre = PaletteChoice::Pierre;
        assert_eq!(
            Theme::styled(kind, ThemeStyle::Grunge, pierre, true, true),
            Theme::new(kind, true, true)
        );
        assert_eq!(
            Theme::styled(kind, ThemeStyle::Punk, pierre, true, true),
            Theme::punk(kind)
        );
        let plain = Theme::styled(kind, ThemeStyle::Plain, pierre, true, true);
        assert_eq!(plain.skin, Skin::Plain);
        assert_eq!(plain.palette, Palette::pierre());
    }

    #[test]
    fn pierre_on_ansi16_is_the_ansi_palette() {
        for style in [ThemeStyle::Grunge, ThemeStyle::Punk, ThemeStyle::Plain] {
            let theme = Theme::styled(ThemeKind::Ansi16, style, PaletteChoice::Pierre, true, true);
            assert_eq!(theme.palette, Palette::ansi16());
        }
    }

    #[test]
    fn ansi16_choice_on_true_color() {
        let kind = ThemeKind::TrueColor;
        let ansi16 = PaletteChoice::Ansi16;
        let grunge = Theme::styled(kind, ThemeStyle::Grunge, ansi16, true, true);
        assert_eq!(grunge.palette, Palette::ansi16());
        assert_eq!(grunge.skin, Skin::Plain);
        let punk = Theme::styled(kind, ThemeStyle::Punk, ansi16, true, true);
        assert_eq!(punk.palette, Palette::ansi16());
        assert_eq!(punk.skin, Skin::Punk);
    }

    #[test]
    fn punk_picks_the_palette_by_kind() {
        let theme = Theme::punk(ThemeKind::TrueColor);
        assert_eq!(theme.palette, Palette::pierre());
        assert_eq!(theme.skin, Skin::Punk);
        let theme = Theme::punk(ThemeKind::Ansi16);
        assert_eq!(theme.palette, Palette::ansi16());
        assert_eq!(theme.skin, Skin::Punk);
        assert!(!theme.textures());
    }

    #[test]
    fn punk_selection_is_a_band_with_a_tail() {
        let theme = Theme::punk(ThemeKind::TrueColor);
        let p = theme.palette;
        let selection = theme.selected_row();
        assert_eq!(selection.patch.bg, Some(p.primary));
        assert_eq!(selection.patch.fg, Some(p.bg));
        assert_eq!(selection.marker, "▶");
        assert_eq!(selection.marker_fg, p.bg);
        assert_eq!(selection.marker_bg, Some(p.fg));
        let [(blue, blue_style), (white, white_style)] = selection.tail.unwrap();
        assert_eq!((blue, white), ("◤", "◤"));
        assert_eq!(
            (blue_style.fg, blue_style.bg),
            (Some(p.primary), Some(p.fg))
        );
        assert_eq!((white_style.fg, white_style.bg), (Some(p.fg), Some(p.bg)));
        assert_eq!(selection.cell_fg, Some(p.bg));
        assert_eq!(selection.shift, 1);
    }

    #[test]
    fn punk_tabs_are_angled() {
        let theme = Theme::punk(ThemeKind::TrueColor);
        let text = |spans: Vec<Span<'_>>| {
            spans
                .iter()
                .map(|s| s.content.to_string())
                .collect::<String>()
        };
        let p = theme.palette;
        assert_eq!(text(theme.tab(" MINE ", true)), "◢ MINE ◤");
        assert_eq!(text(theme.tab(" TEAM ", false)), "◢ TEAM ◤");
        let active = theme.tab(" MINE ", true);
        assert_eq!(active[1].style.bg, Some(p.primary));
        assert_eq!(active[1].style.fg, Some(p.bg));
        assert!(active[1].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(active[0].style.fg, Some(p.primary));
        assert_eq!(active[0].style.bg, Some(p.bg));
        let other = theme.tab(" TEAM ", false);
        assert_eq!(other[1].style.bg, Some(p.fg_mute));
        assert_eq!(other[1].style.fg, Some(p.bg));
        assert_eq!(other[2].style.fg, Some(p.fg_mute));
        assert_eq!(other[2].style.bg, Some(p.bg));
        assert_eq!(theme.tab_title("mine"), "MINE");
    }

    #[test]
    fn punk_tab_strip_shares_the_edges() {
        let theme = Theme::punk(ThemeKind::TrueColor);
        let p = theme.palette;
        let labels = vec![" 1 MINE ".to_owned(), " 2 TEAM ".to_owned()];
        let strip = theme.tab_strip(&labels, 0);
        let text: String = strip.iter().flatten().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "◢ 1 MINE ◤ 2 TEAM ◤");
        let (start, mine, join) = (&strip[0][0], &strip[0][1], &strip[0][2]);
        assert_eq!(
            (start.style.fg, start.style.bg),
            (Some(p.primary), Some(p.bg))
        );
        assert_eq!(
            (mine.style.fg, mine.style.bg),
            (Some(p.bg), Some(p.primary))
        );
        assert_eq!(
            (join.style.fg, join.style.bg),
            (Some(p.primary), Some(p.fg_mute))
        );
        let (team, end) = (&strip[1][0], &strip[1][1]);
        assert_eq!(
            (team.style.fg, team.style.bg),
            (Some(p.bg), Some(p.fg_mute))
        );
        assert_eq!((end.style.fg, end.style.bg), (Some(p.fg_mute), Some(p.bg)));
        // An inactive tab before the active one ends grey on blue.
        let strip = theme.tab_strip(&labels, 1);
        let join = &strip[0][2];
        assert_eq!(
            (join.style.fg, join.style.bg),
            (Some(p.fg_mute), Some(p.primary))
        );
    }

    #[test]
    fn grunge_tab_strip_is_the_tabs() {
        let theme = Theme::new(ThemeKind::TrueColor, true, true);
        let labels = vec![" 1 MINE ".to_owned(), " 2 TEAM ".to_owned()];
        assert_eq!(
            theme.tab_strip(&labels, 1),
            vec![theme.tab(" 1 MINE ", false), theme.tab(" 2 TEAM ", true)]
        );
    }

    #[test]
    fn hint_prefix_keeps_the_width() {
        let punk = Theme::punk(ThemeKind::TrueColor);
        assert_eq!(punk.hint_prefix("▌ ").content, "╱ ");
        let grunge = Theme::new(ThemeKind::TrueColor, true, true);
        assert_eq!(grunge.hint_prefix("▌ ").content, "▌ ");
    }

    #[test]
    fn grunge_tab_title_spaces_the_letters() {
        let theme = Theme::new(ThemeKind::TrueColor, true, true);
        assert_eq!(theme.tab_title("mine"), "M I N E");
    }
}
