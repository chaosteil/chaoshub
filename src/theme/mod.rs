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
use ratatui::widgets::Widget;

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
    /// The background of the selection band, under `fg` text.
    pub band: Color,
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
    /// The glyph at the right end of the row, and its style. A row that
    /// has a tail keeps its last cell free for it.
    pub tail: Option<(&'static str, Style)>,
    /// The text color of every cell on the selected row. `None` keeps the
    /// colors of the cells.
    pub cell_fg: Option<Color>,
}

impl Theme {
    /// Builds the theme for a terminal kind. `grunge` and `textures` come
    /// from the configuration. Without `grunge`, the skin is plain. A
    /// 16-color terminal gets the ANSI palette and the plain skin.
    pub(crate) const fn new(kind: ThemeKind, grunge: bool, textures: bool) -> Self {
        match kind {
            ThemeKind::TrueColor => Self {
                palette: Palette::pierre(),
                skin: if grunge {
                    Skin::Grunge { textures }
                } else {
                    Skin::Plain
                },
            },
            ThemeKind::Ansi16 => Self {
                palette: Palette::ansi16(),
                skin: Skin::Plain,
            },
        }
    }

    /// Builds the punk theme for a terminal kind: the punk palette, or
    /// its 16-color fallback.
    pub(crate) const fn punk(kind: ThemeKind) -> Self {
        Self {
            palette: match kind {
                ThemeKind::TrueColor => Palette::punk(),
                ThemeKind::Ansi16 => Palette::punk_ansi16(),
            },
            skin: Skin::Punk,
        }
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

    /// Fills the empty cells of an area. `seed` picks the pattern. Only
    /// the grunge skin with textures draws a fill.
    pub(crate) fn fill(&self, area: Rect, buf: &mut Buffer, seed: u64) {
        Texture::new(self).seed(seed).render(area, buf);
    }

    /// How the selected row of a list looks. Grunge and plain rows get
    /// the surface background and a `▌` marker. A punk row is a slanted
    /// band: `▶`, the cells in white on the band red, and a `╱` tail.
    pub(crate) fn selected_row(&self) -> Selection {
        let p = &self.palette;
        match self.skin {
            Skin::Punk => Selection {
                patch: Style::new().bg(p.band).fg(p.fg),
                marker: "▶",
                marker_fg: p.fg,
                tail: Some((
                    "╱",
                    Style::new()
                        .bg(p.bg)
                        .fg(p.band)
                        .add_modifier(Modifier::BOLD),
                )),
                cell_fg: Some(p.fg),
            },
            Skin::Grunge { .. } | Skin::Plain => Selection {
                patch: Style::new().bg(p.surface),
                marker: "▌",
                marker_fg: p.primary,
                tail: None,
                cell_fg: None,
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
    /// A punk tab is angled, `◤ label ◢`, and the active one is filled
    /// with the accent red. The other punk tabs are plain.
    pub(crate) fn tab(&self, label: &str, active: bool) -> Vec<Span<'static>> {
        let p = &self.palette;
        if self.skin == Skin::Punk {
            let edge = Style::new().bg(p.bg).fg(p.primary);
            return if active {
                vec![
                    Span::styled("◤", edge),
                    Span::styled(
                        label.to_owned(),
                        Style::new()
                            .bg(p.primary)
                            .fg(p.bg)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("◢", edge),
                ]
            } else {
                let style = Style::new().bg(p.bg).fg(p.fg_dim);
                vec![
                    Span::styled(" ", style),
                    Span::styled(label.to_owned(), style),
                    Span::styled(" ", style),
                ]
            };
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

    /// How the status bar looks. Grunge and plain bars have the surface
    /// background. The punk bar has a red band with black text on the
    /// left, a diagonal end, and the message in white.
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
    fn punk_picks_the_palette_by_kind() {
        let theme = Theme::punk(ThemeKind::TrueColor);
        assert_eq!(theme.palette, Palette::punk());
        assert_eq!(theme.skin, Skin::Punk);
        let theme = Theme::punk(ThemeKind::Ansi16);
        assert_eq!(theme.palette, Palette::punk_ansi16());
        assert!(!theme.textures());
    }

    #[test]
    fn punk_selection_is_a_band_with_a_tail() {
        let theme = Theme::punk(ThemeKind::TrueColor);
        let p = theme.palette;
        let selection = theme.selected_row();
        assert_eq!(selection.patch.bg, Some(p.band));
        assert_eq!(selection.patch.fg, Some(p.fg));
        assert_eq!(selection.marker, "▶");
        assert_eq!(selection.tail.map(|(glyph, _)| glyph), Some("╱"));
        assert_eq!(selection.cell_fg, Some(p.fg));
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
        assert_eq!(text(theme.tab(" MINE ", true)), "◤ MINE ◢");
        assert_eq!(text(theme.tab(" TEAM ", false)), "  TEAM  ");
        assert_eq!(
            theme.tab(" MINE ", true)[1].style.bg,
            Some(theme.palette.primary)
        );
        assert_eq!(theme.tab_title("mine"), "MINE");
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
