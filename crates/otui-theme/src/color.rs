//! Color string parsing.
//!
//! Theme slots are stored as strings so a user can drop a TOML theme into their
//! config without obsidian-tui needing to know about it at compile time. A slot
//! is one of:
//!
//! - a hex color, `#rrggbb` or the short `#rgb` form
//! - one of the terminal's own ANSI color names (`red`, `brightblue`, `gray`, …)
//! - `reset`, meaning "inherit whatever the terminal itself uses"
//!
//! `reset` is what makes the `terminal` theme work: rather than imposing a
//! palette, it defers to the colors the user already configured in their
//! terminal emulator, so obsidian-tui never clashes with the rest of their setup.

use ratatui::style::Color;

/// Parses a theme color string. Anything unrecognized resolves to
/// [`Color::Reset`] rather than panicking, so a typo in a user's theme file
/// degrades to "terminal default" instead of taking the app down.
pub fn parse(s: &str) -> Color {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        return parse_hex(hex).unwrap_or(Color::Reset);
    }

    match s.to_ascii_lowercase().as_str() {
        "reset" | "none" | "" => Color::Reset,
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" | "purple" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "lightred" | "brightred" => Color::LightRed,
        "lightgreen" | "brightgreen" => Color::LightGreen,
        "lightyellow" | "brightyellow" => Color::LightYellow,
        "lightblue" | "brightblue" => Color::LightBlue,
        "lightmagenta" | "brightmagenta" => Color::LightMagenta,
        "lightcyan" | "brightcyan" => Color::LightCyan,
        other => other.parse::<u8>().map_or(Color::Reset, Color::Indexed),
    }
}

fn parse_hex(hex: &str) -> Option<Color> {
    let bytes = hex.as_bytes();
    match bytes.len() {
        // #rgb — each nibble is doubled, so `#f0a` means `#ff00aa`.
        3 => {
            let r = nibble(bytes[0])?;
            let g = nibble(bytes[1])?;
            let b = nibble(bytes[2])?;
            Some(Color::Rgb(r * 17, g * 17, b * 17))
        }
        6 => {
            let r = byte(bytes[0], bytes[1])?;
            let g = byte(bytes[2], bytes[3])?;
            let b = byte(bytes[4], bytes[5])?;
            Some(Color::Rgb(r, g, b))
        }
        _ => None,
    }
}

fn nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn byte(hi: u8, lo: u8) -> Option<u8> {
    Some(nibble(hi)? * 16 + nibble(lo)?)
}

/// Blends `a` toward `b` by `t` (0.0 → `a`, 1.0 → `b`).
///
/// Only meaningful for two true-color values; if either side is an ANSI or
/// `reset` color there is nothing to interpolate between, so `a` is returned
/// unchanged. Used to derive hover/selection tints from a theme's base colors.
pub fn blend(a: Color, b: Color, t: f32) -> Color {
    let (Color::Rgb(ar, ag, ab), Color::Rgb(br, bg, bb)) = (a, b) else {
        return a;
    };
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color::Rgb(mix(ar, br), mix(ag, bg), mix(ab, bb))
}

/// Perceived luminance in 0.0..=1.0, or `None` for non-RGB colors.
///
/// Used to decide whether text drawn on top of a color should be light or dark.
pub fn luminance(color: Color) -> Option<f32> {
    let Color::Rgb(r, g, b) = color else {
        return None;
    };
    Some((0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b)) / 255.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_forms() {
        assert_eq!(parse("#1e1e1e"), Color::Rgb(0x1e, 0x1e, 0x1e));
        assert_eq!(parse("#f0a"), Color::Rgb(0xff, 0x00, 0xaa));
        assert_eq!(parse("  #8b6cef  "), Color::Rgb(0x8b, 0x6c, 0xef));
    }

    #[test]
    fn parses_names_case_insensitively() {
        assert_eq!(parse("Red"), Color::Red);
        assert_eq!(parse("darkgrey"), Color::DarkGray);
        assert_eq!(parse("brightblue"), Color::LightBlue);
        assert_eq!(parse("42"), Color::Indexed(42));
    }

    #[test]
    fn unknown_and_empty_fall_back_to_reset() {
        assert_eq!(parse(""), Color::Reset);
        assert_eq!(parse("reset"), Color::Reset);
        assert_eq!(parse("chartreuse"), Color::Reset);
        assert_eq!(parse("#12345"), Color::Reset);
    }

    #[test]
    fn blend_interpolates_rgb_only() {
        let black = Color::Rgb(0, 0, 0);
        let white = Color::Rgb(255, 255, 255);
        assert_eq!(blend(black, white, 0.5), Color::Rgb(128, 128, 128));
        assert_eq!(blend(black, white, 0.0), black);
        // Nothing to interpolate against a terminal-default color.
        assert_eq!(blend(Color::Reset, white, 0.5), Color::Reset);
    }
}
