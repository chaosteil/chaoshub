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
    /// The rows between the tab bar and the key hint bar.
    pub list: Rect,
    /// The key hint bar: the row above the status bar. Empty when the
    /// key hints are off.
    pub hints: Rect,
    /// The last row.
    pub status: Rect,
    /// The PR popup: a sidebar at the right edge, 80% of the width, over
    /// the list rows.
    pub popup: Rect,
    /// The pane inside the PR popup: the popup without its border, its
    /// side padding and its header rows. `Pane::model` gets this size.
    pub popup_body: Rect,
    /// The help overlay: at the right edge, over the list rows.
    pub help: Rect,
}

/// The share of the width for the popup, in percent.
const POPUP_PERCENT: u16 = 80;
/// The share of the width for the help overlay, in percent.
const HELP_PERCENT: u16 = 40;
/// The minimum width of the help overlay.
const HELP_MIN_WIDTH: u16 = 40;
/// The width of the popup border on each side, as `ui/popup.rs` draws it.
const POPUP_BORDER: u16 = 1;
/// The blank column inside the popup border on the left and the right.
const POPUP_PADDING: u16 = 1;
/// The popup rows above the pane: title, subtitle, pane tabs and a rule.
const POPUP_HEADER_ROWS: u16 = 4;

/// `pct` percent of `value`, rounded down.
fn percent(value: u16, pct: u16) -> u16 {
    u16::try_from(u32::from(value) * u32::from(pct) / 100).unwrap_or(value)
}

/// Computes the areas for a terminal size, with the key hint bar.
pub(crate) fn layout(size: Size) -> Areas {
    layout_with(size, true)
}

/// Computes the areas for a terminal size. `key_hints` is the
/// `[theme] key_hints` switch: when it is false, the key hint bar is
/// empty and the list takes its row.
pub(crate) fn layout_with(size: Size, key_hints: bool) -> Areas {
    layout_full(size, key_hints, false)
}

/// Computes the areas for a terminal size. `key_hints` is the
/// `[theme] key_hints` switch. `pinned` is true in the pinned mode: the
/// tab bar is empty, the list starts at the first row, and the popup
/// covers the whole list area.
pub(crate) fn layout_full(size: Size, key_hints: bool, pinned: bool) -> Areas {
    let Size { width, height } = size;
    let tab_bar = Rect {
        x: 0,
        y: 0,
        width,
        height: if pinned { 0 } else { height.min(1) },
    };
    let status = Rect {
        x: 0,
        y: height.saturating_sub(1),
        width,
        height: height.saturating_sub(1).min(1),
    };
    let hints = if key_hints {
        Rect {
            x: 0,
            y: status.y.saturating_sub(1),
            width,
            height: height.saturating_sub(2).min(1),
        }
    } else {
        Rect::default()
    };
    let list = Rect {
        x: 0,
        y: tab_bar.height,
        width,
        height: height.saturating_sub(1 + tab_bar.height + hints.height),
    };
    let popup = if pinned {
        list
    } else {
        let popup_width = percent(width, POPUP_PERCENT);
        Rect {
            x: width - popup_width,
            y: list.y,
            width: popup_width,
            height: list.height,
        }
    };
    let popup_body = Rect {
        x: popup.x.saturating_add(POPUP_BORDER + POPUP_PADDING),
        y: popup.y.saturating_add(POPUP_BORDER + POPUP_HEADER_ROWS),
        width: popup
            .width
            .saturating_sub(2 * (POPUP_BORDER + POPUP_PADDING)),
        height: popup
            .height
            .saturating_sub(2 * POPUP_BORDER + POPUP_HEADER_ROWS),
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
        hints,
        status,
        popup,
        popup_body,
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
                list: rect(0, 1, 100, 27),
                hints: rect(0, 28, 100, 1),
                status: rect(0, 29, 100, 1),
                popup: rect(20, 1, 80, 27),
                popup_body: rect(22, 6, 76, 21),
                help: rect(60, 1, 40, 27),
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
                list: rect(0, 1, 160, 47),
                hints: rect(0, 48, 160, 1),
                status: rect(0, 49, 160, 1),
                popup: rect(32, 1, 128, 47),
                popup_body: rect(34, 6, 124, 41),
                help: rect(96, 1, 64, 47),
            }
        );
    }

    #[test]
    fn without_key_hints_the_list_takes_the_hints_row() {
        let size = Size {
            width: 100,
            height: 30,
        };
        let areas = layout_with(size, false);
        assert_eq!(areas.hints, Rect::default());
        assert_eq!(areas.list, rect(0, 1, 100, 28));
        assert_eq!(areas.help, rect(60, 1, 40, 28));
        assert_eq!(areas.popup, rect(20, 1, 80, 28));
        assert_eq!(areas.popup_body, rect(22, 6, 76, 22));
        assert_eq!(
            Areas {
                list: areas.list,
                hints: areas.hints,
                popup: areas.popup,
                popup_body: areas.popup_body,
                help: areas.help,
                ..layout(size)
            },
            areas
        );
    }

    #[test]
    fn pinned_the_popup_covers_the_screen_above_the_hints() {
        let areas = layout_full(
            Size {
                width: 100,
                height: 30,
            },
            true,
            true,
        );
        assert_eq!(areas.tab_bar.height, 0);
        assert_eq!(areas.list, rect(0, 0, 100, 28));
        assert_eq!(areas.hints, rect(0, 28, 100, 1));
        assert_eq!(areas.status, rect(0, 29, 100, 1));
        assert_eq!(areas.popup, rect(0, 0, 100, 28));
        assert_eq!(areas.popup_body, rect(2, 5, 96, 22));
    }

    #[test]
    fn tiny_screens_have_no_negative_rows() {
        for height in 0..4 {
            let areas = layout(Size { width: 10, height });
            let rows =
                areas.tab_bar.height + areas.list.height + areas.hints.height + areas.status.height;
            assert_eq!(rows, height, "height {height}");
        }
    }
}
