//! The screen areas, as plain rects. `app` and `ui` both use them, so
//! they never disagree.
#![allow(dead_code, reason = "app and ui use the layout in later phases")]

use crate::bus::Size;

/// A rectangle of terminal cells. Plain data, not the ratatui type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(crate) struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

/// The areas of the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Areas {
    /// The first row.
    pub tab_bar: Rect,
    /// The rows between the tab bar and the status bar.
    pub list: Rect,
    /// The last row.
    pub status: Rect,
    /// The PR popup: centered, 85% of the width and of the height.
    pub popup: Rect,
    /// The help overlay: at the right edge, over the list rows.
    pub help: Rect,
}

/// The share of the screen for the popup, in percent.
const POPUP_PERCENT: u16 = 85;
/// The share of the width for the help overlay, in percent.
const HELP_PERCENT: u16 = 40;
/// The minimum width of the help overlay.
const HELP_MIN_WIDTH: u16 = 40;

/// `pct` percent of `value`, rounded down.
fn percent(value: u16, pct: u16) -> u16 {
    u16::try_from(u32::from(value) * u32::from(pct) / 100).unwrap_or(value)
}

/// Computes the areas for a terminal size.
pub(crate) fn layout(size: Size) -> Areas {
    let Size { width, height } = size;
    let tab_bar = Rect {
        x: 0,
        y: 0,
        width,
        height: height.min(1),
    };
    let status = Rect {
        x: 0,
        y: height.saturating_sub(1),
        width,
        height: height.saturating_sub(1).min(1),
    };
    let list = Rect {
        x: 0,
        y: tab_bar.height,
        width,
        height: height.saturating_sub(2),
    };
    let popup_width = percent(width, POPUP_PERCENT);
    let popup_height = percent(height, POPUP_PERCENT);
    let popup = Rect {
        x: (width - popup_width) / 2,
        y: (height - popup_height) / 2,
        width: popup_width,
        height: popup_height,
    };
    let help_width = percent(width, HELP_PERCENT).max(HELP_MIN_WIDTH).min(width);
    let help = Rect {
        x: width - help_width,
        y: list.y,
        width: help_width,
        height: list.height,
    };
    Areas {
        tab_bar,
        list,
        status,
        popup,
        help,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u16, y: u16, width: u16, height: u16) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn layout_100x30() {
        let areas = layout(Size {
            width: 100,
            height: 30,
        });
        assert_eq!(
            areas,
            Areas {
                tab_bar: rect(0, 0, 100, 1),
                list: rect(0, 1, 100, 28),
                status: rect(0, 29, 100, 1),
                popup: rect(7, 2, 85, 25),
                help: rect(60, 1, 40, 28),
            }
        );
    }

    #[test]
    fn layout_160x50() {
        let areas = layout(Size {
            width: 160,
            height: 50,
        });
        assert_eq!(
            areas,
            Areas {
                tab_bar: rect(0, 0, 160, 1),
                list: rect(0, 1, 160, 48),
                status: rect(0, 49, 160, 1),
                popup: rect(12, 4, 136, 42),
                help: rect(96, 1, 64, 48),
            }
        );
    }
}
