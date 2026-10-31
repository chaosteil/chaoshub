//! The theme: palette, grunge borders and textures.
#![allow(dead_code, reason = "ui and run use the theme in later phases")]

pub(crate) mod grunge;
mod palette;
pub(crate) mod texture;

use ratatui::style::Color;

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

/// The palette and the style switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Theme {
    pub palette: Palette,
    /// True for the grunge borders.
    pub grunge: bool,
    /// True for the textures in empty areas.
    pub textures: bool,
}

impl Theme {
    /// Builds the theme for a terminal kind. `grunge` and `textures` come
    /// from the configuration. A 16-color terminal gets the ANSI palette,
    /// rounded borders and no textures.
    pub(crate) const fn new(kind: ThemeKind, grunge: bool, textures: bool) -> Self {
        match kind {
            ThemeKind::TrueColor => Self {
                palette: Palette::pierre(),
                grunge,
                textures,
            },
            ThemeKind::Ansi16 => Self {
                palette: Palette::ansi16(),
                grunge: false,
                textures: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn true_color_keeps_the_switches() {
        let theme = Theme::new(ThemeKind::TrueColor, true, false);
        assert_eq!(theme.palette, Palette::pierre());
        assert!(theme.grunge);
        assert!(!theme.textures);
    }

    #[test]
    fn ansi16_turns_grunge_and_textures_off() {
        let theme = Theme::new(ThemeKind::Ansi16, true, true);
        assert_eq!(theme.palette, Palette::ansi16());
        assert!(!theme.grunge);
        assert!(!theme.textures);
    }
}
