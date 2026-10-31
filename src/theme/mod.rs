//! The theme: palette, skin, borders and textures.
#![allow(dead_code, reason = "ui and run use the theme in later phases")]

mod block;
pub(crate) mod grunge;
mod palette;
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

    /// A framed block with a header on the top edge. An empty `title`
    /// gives a block without a header.
    pub(crate) fn block<'a>(&self, title: &'a str) -> ThemeBlock<'a> {
        ThemeBlock::new(self, title)
    }

    /// A header line in the color `color`.
    #[allow(
        clippy::unused_self,
        reason = "the punk skin reads the skin in a later commit"
    )]
    pub(crate) fn header(&self, text: &str, color: Color) -> Line<'static> {
        stencil(text, Style::new().fg(color))
    }

    /// A horizontal rule of `width` cells.
    #[allow(
        clippy::unused_self,
        reason = "the punk skin reads the skin in a later commit"
    )]
    pub(crate) fn separator(&self, width: usize, style: Style) -> Span<'static> {
        Span::styled("─".repeat(width), style)
    }

    /// Fills the empty cells of an area. `seed` picks the pattern. Only
    /// the grunge skin with textures draws a fill.
    pub(crate) fn fill(&self, area: Rect, buf: &mut Buffer, seed: u64) {
        Texture::new(self).seed(seed).render(area, buf);
    }

    /// How the selected row of a list looks.
    pub(crate) fn selected_row(&self) -> Selection {
        let p = &self.palette;
        Selection {
            patch: Style::new().bg(p.surface),
            marker: "▌",
            marker_fg: p.primary,
            tail: None,
            cell_fg: None,
        }
    }

    /// The title of a tab in the tab bar.
    #[allow(
        clippy::unused_self,
        reason = "the punk skin reads the skin in a later commit"
    )]
    pub(crate) fn tab_title(&self, title: &str) -> String {
        title
            .to_uppercase()
            .chars()
            .map(String::from)
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The spans of one tab with its label. The active tab stands out.
    pub(crate) fn tab(&self, label: &str, active: bool) -> Vec<Span<'static>> {
        let p = &self.palette;
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
    fn grunge_tab_title_spaces_the_letters() {
        let theme = Theme::new(ThemeKind::TrueColor, true, true);
        assert_eq!(theme.tab_title("mine"), "M I N E");
    }
}
