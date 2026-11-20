//! Writes a ratatui buffer as an SVG image of a terminal window.
//!
//! Each cell is a box of `CELL_W` x `CELL_H` pixels. The background of
//! each run of cells with the same color is one rect. The text of each
//! run of ASCII cells with the same style is one `<text>` element, with a
//! `textLength` that fits the run to its cells. Each other symbol is one
//! `<text>` element at the center of its cells, so a font with other
//! widths cannot move the columns. The block, triangle and box drawing
//! symbols of the frames are shapes, so they join without gaps in every
//! font.

use std::fmt::Write as _;

use ratatui::buffer::{Buffer, Cell};
use ratatui::style::{Color, Modifier};
use ratatui::text::Span;

/// The width of a cell, in pixels.
const CELL_W: f64 = 8.4;
/// The height of a cell, in pixels.
const CELL_H: f64 = 18.0;
/// The font size, in pixels.
const FONT_SIZE: f64 = 14.0;
/// The distance from the top of a cell to the baseline of its text.
const BASELINE: f64 = 13.5;
/// The thickness of a light and a heavy box drawing line.
const LIGHT: f64 = 1.4;
const HEAVY: f64 = 2.4;
/// The space between the window edge and the cells.
const PAD: f64 = 14.0;
/// The height of the title bar of the window.
const BAR_H: f64 = 32.0;
/// The space around the window, for its shadow.
const MARGIN: f64 = 20.0;
/// The fonts of the text. The first font that the viewer has wins.
const FONTS: &str = "ui-monospace, SFMono-Regular, Menlo, Consolas, 'DejaVu Sans Mono', \
                     'Liberation Mono', 'Noto Sans Mono', monospace";

/// The resolved style of one cell.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Look {
    fg: Rgb,
    bg: Rgb,
    bold: bool,
    italic: bool,
    dim: bool,
    underline: bool,
    strike: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rgb(u8, u8, u8);

impl std::fmt::Display for Rgb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

/// The colors of the window: the text and background colors for
/// `Color::Reset`, and the color of the title bar.
pub(super) struct Colors {
    pub fg: Color,
    pub bg: Color,
    pub bar: Color,
}

/// The SVG image of `buf` in a window with the title `title`.
pub(super) fn render(buf: &Buffer, title: &str, colors: &Colors) -> String {
    let fg = rgb(colors.fg, Rgb(0xfa, 0xfa, 0xfa));
    let bg = rgb(colors.bg, Rgb(0x0a, 0x0a, 0x0a));
    let bar = rgb(colors.bar, Rgb(0x17, 0x17, 0x17));
    let cols = buf.area.width;
    let rows = buf.area.height;
    let win_w = f64::from(cols) * CELL_W + 2.0 * PAD;
    let win_h = f64::from(rows) * CELL_H + 2.0 * PAD + BAR_H;
    let width = win_w + 2.0 * MARGIN;
    let height = win_h + 2.0 * MARGIN;
    let mut out = String::new();
    let _ = writeln!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0}" height="{height:.0}" viewBox="0 0 {width:.1} {height:.1}">"#
    );
    let _ = writeln!(
        out,
        r##"<defs><filter id="shadow" x="-5%" y="-5%" width="110%" height="115%"><feDropShadow dx="0" dy="4" stdDeviation="6" flood-color="#000" flood-opacity="0.45"/></filter><clipPath id="window"><rect x="{MARGIN}" y="{MARGIN}" width="{win_w:.1}" height="{win_h:.1}" rx="10"/></clipPath></defs>"##
    );
    let _ = writeln!(
        out,
        r##"<rect x="{MARGIN}" y="{MARGIN}" width="{win_w:.1}" height="{win_h:.1}" rx="10" fill="{bg}" stroke="#2a2a2a" filter="url(#shadow)"/>"##
    );
    let _ = writeln!(
        out,
        r#"<g clip-path="url(#window)"><rect x="{MARGIN}" y="{MARGIN}" width="{win_w:.1}" height="{BAR_H}" fill="{bar}"/></g>"#
    );
    for (i, color) in ["#ff5f57", "#febc2e", "#28c840"].iter().enumerate() {
        let cx = MARGIN + 20.0 + 20.0 * f64::from(u8::try_from(i).unwrap_or(0));
        let cy = MARGIN + BAR_H / 2.0;
        let _ = writeln!(
            out,
            r#"<circle cx="{cx:.1}" cy="{cy:.1}" r="6" fill="{color}"/>"#
        );
    }
    let _ = writeln!(
        out,
        r##"<text x="{:.1}" y="{:.1}" fill="#a3a3a3" font-family="{FONTS}" font-size="13" text-anchor="middle">{}</text>"##,
        MARGIN + win_w / 2.0,
        MARGIN + BAR_H / 2.0 + 4.5,
        escape(title)
    );
    let left = MARGIN + PAD;
    let top = MARGIN + BAR_H + PAD;
    let _ = writeln!(
        out,
        r#"<g transform="translate({left:.1} {top:.1})" font-family="{FONTS}" font-size="{FONT_SIZE}">"#
    );
    let looks: Vec<Vec<Look>> = (0..rows)
        .map(|y| (0..cols).map(|x| look(&buf[(x, y)], fg, bg)).collect())
        .collect();
    backgrounds(&mut out, &looks, bg);
    for y in 0..rows {
        row_text(&mut out, buf, &looks[usize::from(y)], y);
    }
    out.push_str("</g>\n</svg>\n");
    out
}

/// The rects of the backgrounds that differ from the window background.
fn backgrounds(out: &mut String, looks: &[Vec<Look>], bg: Rgb) {
    for (y, row) in looks.iter().enumerate() {
        let mut x = 0;
        while x < row.len() {
            let color = row[x].bg;
            let start = x;
            while x < row.len() && row[x].bg == color {
                x += 1;
            }
            if color != bg {
                let _ = writeln!(
                    out,
                    r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{CELL_H}" fill="{color}"/>"#,
                    px(start) * CELL_W,
                    px(y) * CELL_H,
                    px(x - start) * CELL_W,
                );
            }
        }
    }
}

/// The text and the shapes of row `y`.
fn row_text(out: &mut String, buf: &Buffer, looks: &[Look], y: u16) {
    let cols = buf.area.width;
    let top = f64::from(y) * CELL_H;
    let mut x: u16 = 0;
    while x < cols {
        let cell = &buf[(x, y)];
        let symbol = cell.symbol();
        let look = looks[usize::from(x)];
        let width = u16::try_from(Span::raw(symbol).width()).unwrap_or(1).max(1);
        if cell.modifier.contains(Modifier::HIDDEN) || symbol.trim().is_empty() {
            x += width;
            continue;
        }
        let left = f64::from(x) * CELL_W;
        // A run of one horizontal line symbol is one rect, so the line
        // has no seams between the cells.
        if let Some(t) = line_width(symbol) {
            let start = x;
            while x < cols && buf[(x, y)].symbol() == symbol && looks[usize::from(x)].fg == look.fg
            {
                x += 1;
            }
            let _ = writeln!(
                out,
                r#"<rect x="{left:.2}" y="{:.2}" width="{:.2}" height="{t}" fill="{}"/>"#,
                top + CELL_H / 2.0 - t / 2.0,
                f64::from(x - start) * CELL_W,
                look.fg,
            );
            continue;
        }
        if shape(out, symbol, left, top, look.fg) {
            x += 1;
            continue;
        }
        if is_ascii(symbol) {
            let start = x;
            let mut text = String::new();
            while x < cols {
                let next = &buf[(x, y)];
                if looks[usize::from(x)] != look
                    || !is_ascii(next.symbol())
                    || next.modifier.contains(Modifier::HIDDEN)
                {
                    break;
                }
                text.push_str(next.symbol());
                x += 1;
            }
            // Trailing spaces need no text.
            let trimmed = text.trim_end();
            let len = trimmed.chars().count();
            let _ = writeln!(
                out,
                r#"<text x="{left:.1}" y="{:.1}" textLength="{:.1}" lengthAdjust="spacingAndGlyphs" xml:space="preserve"{}>{}</text>"#,
                top + BASELINE,
                px(len) * CELL_W,
                attrs(look),
                escape(trimmed),
            );
            debug_assert!(x > start);
            continue;
        }
        let center = left + f64::from(width) * CELL_W / 2.0;
        let _ = writeln!(
            out,
            r#"<text x="{center:.1}" y="{:.1}" text-anchor="middle"{}>{}</text>"#,
            top + BASELINE,
            attrs(look),
            escape(symbol),
        );
        x += width;
    }
}

/// True for one printable ASCII character, a space included.
fn is_ascii(symbol: &str) -> bool {
    let mut chars = symbol.chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if c.is_ascii() && !c.is_ascii_control())
}

/// The fill and font attributes of a style.
fn attrs(look: Look) -> String {
    let mut attrs = format!(r#" fill="{}""#, look.fg);
    if look.bold {
        attrs.push_str(r#" font-weight="bold""#);
    }
    if look.italic {
        attrs.push_str(r#" font-style="italic""#);
    }
    if look.dim {
        attrs.push_str(r#" fill-opacity="0.6""#);
    }
    match (look.underline, look.strike) {
        (true, true) => attrs.push_str(r#" text-decoration="underline line-through""#),
        (true, false) => attrs.push_str(r#" text-decoration="underline""#),
        (false, true) => attrs.push_str(r#" text-decoration="line-through""#),
        (false, false) => {}
    }
    attrs
}

/// Draws `symbol` as a shape in the cell at `left`, `top`. False when
/// the symbol has no shape.
fn shape(out: &mut String, symbol: &str, left: f64, top: f64, fg: Rgb) -> bool {
    let (x0, y0, x1, y1) = (left, top, left + CELL_W, top + CELL_H);
    let (xm, ym) = (left + CELL_W / 2.0, top + CELL_H / 2.0);
    let polygon = |out: &mut String, points: &[(f64, f64)]| {
        let points: Vec<String> = points
            .iter()
            .map(|(x, y)| format!("{x:.1},{y:.1}"))
            .collect();
        let _ = writeln!(
            out,
            r#"<polygon points="{}" fill="{fg}"/>"#,
            points.join(" ")
        );
    };
    let rect = |out: &mut String, x: f64, y: f64, w: f64, h: f64| {
        let _ = writeln!(
            out,
            r#"<rect x="{x:.2}" y="{y:.2}" width="{w:.2}" height="{h:.2}" fill="{fg}"/>"#
        );
    };
    match symbol {
        "█" => rect(out, x0, y0, CELL_W, CELL_H),
        "▌" => rect(out, x0, y0, CELL_W / 2.0, CELL_H),
        "▐" => rect(out, xm, y0, CELL_W / 2.0, CELL_H),
        "▀" => rect(out, x0, y0, CELL_W, CELL_H / 2.0),
        "▄" => rect(out, x0, ym, CELL_W, CELL_H / 2.0),
        "◢" => polygon(out, &[(x1, y0), (x1, y1), (x0, y1)]),
        "◣" => polygon(out, &[(x0, y0), (x1, y1), (x0, y1)]),
        "◤" => polygon(out, &[(x0, y0), (x1, y0), (x0, y1)]),
        "◥" => polygon(out, &[(x0, y0), (x1, y0), (x1, y1)]),
        "╱" => polygon(out, &[(x1 - 1.0, y0), (x1, y0), (x0 + 1.0, y1), (x0, y1)]),
        "╲" => polygon(out, &[(x0, y0), (x0 + 1.0, y0), (x1, y1), (x1 - 1.0, y1)]),
        _ => {
            let Some((heavy, [left_arm, right_arm, up, down])) = arms(symbol) else {
                return false;
            };
            let thick = if heavy { HEAVY } else { LIGHT };
            let half = thick / 2.0;
            // Each arm goes from the edge to the far side of the center
            // line, so a corner has no notch.
            if left_arm {
                rect(out, x0, ym - half, CELL_W / 2.0 + half, thick);
            }
            if right_arm {
                rect(out, xm - half, ym - half, CELL_W / 2.0 + half, thick);
            }
            if up {
                rect(out, xm - half, y0, thick, CELL_H / 2.0 + half);
            }
            if down {
                rect(out, xm - half, ym - half, thick, CELL_H / 2.0 + half);
            }
        }
    }
    true
}

/// The thickness of a horizontal line symbol. `None` for other
/// symbols.
fn line_width(symbol: &str) -> Option<f64> {
    match symbol {
        "─" => Some(LIGHT),
        "━" => Some(HEAVY),
        _ => None,
    }
}

/// The arms of a box drawing symbol: heavy or light, and left, right, up
/// and down.
fn arms(symbol: &str) -> Option<(bool, [bool; 4])> {
    let arms = match symbol {
        "─" => (false, [true, true, false, false]),
        "│" => (false, [false, false, true, true]),
        "┌" => (false, [false, true, false, true]),
        "┐" => (false, [true, false, false, true]),
        "└" => (false, [false, true, true, false]),
        "┘" => (false, [true, false, true, false]),
        "├" => (false, [false, true, true, true]),
        "┤" => (false, [true, false, true, true]),
        "┬" => (false, [true, true, false, true]),
        "┴" => (false, [true, true, true, false]),
        "┼" => (false, [true, true, true, true]),
        "━" => (true, [true, true, false, false]),
        "┃" => (true, [false, false, true, true]),
        "┏" => (true, [false, true, false, true]),
        "┓" => (true, [true, false, false, true]),
        "┗" => (true, [false, true, true, false]),
        "┛" => (true, [true, false, true, false]),
        "┣" => (true, [false, true, true, true]),
        "┫" => (true, [true, false, true, true]),
        "┳" => (true, [true, true, false, true]),
        "┻" => (true, [true, true, true, false]),
        "╋" => (true, [true, true, true, true]),
        _ => return None,
    };
    Some(arms)
}

/// The look of a cell, with `fg` and `bg` for `Color::Reset`.
fn look(cell: &Cell, fg: Rgb, bg: Rgb) -> Look {
    let mut fore = rgb(cell.fg, fg);
    let mut back = rgb(cell.bg, bg);
    let m = cell.modifier;
    if m.contains(Modifier::REVERSED) {
        std::mem::swap(&mut fore, &mut back);
    }
    Look {
        fg: fore,
        bg: back,
        bold: m.contains(Modifier::BOLD),
        italic: m.contains(Modifier::ITALIC),
        dim: m.contains(Modifier::DIM),
        underline: m.contains(Modifier::UNDERLINED),
        strike: m.contains(Modifier::CROSSED_OUT),
    }
}

/// The RGB value of a color, with `reset` for `Color::Reset`. The 16
/// ANSI colors get the colors of a dark terminal theme.
fn rgb(color: Color, reset: Rgb) -> Rgb {
    match color {
        Color::Reset | Color::Indexed(_) => reset,
        Color::Rgb(r, g, b) => Rgb(r, g, b),
        Color::Black => Rgb(0x0a, 0x0a, 0x0a),
        Color::Red => Rgb(0xcd, 0x31, 0x31),
        Color::Green => Rgb(0x0d, 0xbc, 0x79),
        Color::Yellow => Rgb(0xe5, 0xe5, 0x10),
        Color::Blue => Rgb(0x24, 0x72, 0xc8),
        Color::Magenta => Rgb(0xbc, 0x3f, 0xbc),
        Color::Cyan => Rgb(0x11, 0xa8, 0xcd),
        Color::Gray => Rgb(0xd4, 0xd4, 0xd4),
        Color::DarkGray => Rgb(0x66, 0x66, 0x66),
        Color::LightRed => Rgb(0xf1, 0x4c, 0x4c),
        Color::LightGreen => Rgb(0x23, 0xd1, 0x8b),
        Color::LightYellow => Rgb(0xf5, 0xf5, 0x43),
        Color::LightBlue => Rgb(0x3b, 0x8e, 0xea),
        Color::LightMagenta => Rgb(0xd6, 0x70, 0xd6),
        Color::LightCyan => Rgb(0x29, 0xb8, 0xdb),
        Color::White => Rgb(0xfa, 0xfa, 0xfa),
    }
}

/// A count of cells as a float.
fn px(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// The text with the XML special characters escaped.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}
