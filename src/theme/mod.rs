//! The theme: palette, grunge borders and textures.
#![allow(dead_code, reason = "ui and run use the theme in later phases")]

mod grunge;
mod palette;
mod texture;

use ratatui::style::Color;

/// The color support of the terminal, which `main` detects from
/// `COLORTERM` and `TERM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ThemeKind {
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
