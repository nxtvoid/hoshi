//! Color math for themes (`lib/color.ts`): parsing CSS colors, OKLCH, contrast.

use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oklch {
    pub l: f64,
    pub c: f64,
    pub h: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParsedColor {
    pub color: Oklch,
    pub alpha: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsv {
    pub h: f64,
    pub s: f64,
    pub v: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightnessDirection {
    Lighter,
    Darker,
}

pub const WHITE: Rgb = Rgb::new(255.0, 255.0, 255.0);
pub const BLACK: Rgb = Rgb::new(0.0, 0.0, 0.0);

/// At this luminance black and white text have the same contrast.
const DARK_LUMINANCE: f64 = 0.179;

impl Rgb {
    pub const fn new(r: f64, g: f64, b: f64) -> Self {
        Self { r, g, b }
    }
}

fn clamp(value: f64, min: f64, max: f64) -> f64 {
    max.min(min.max(value))
}

/// `Math.round`: halves go up.
pub(crate) fn js_round(value: f64) -> f64 {
    let floor = value.floor();
    if value - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }
}

/// `Number.prototype.toFixed`, which rounds exact ties away from zero.
pub(crate) fn to_fixed(value: f64, precision: usize) -> String {
    let magnitude = value.abs();
    let exact = format!("{magnitude:.prec$}", prec = precision + 25);
    let decimals = exact.split_once('.').map_or("", |(_, digits)| digits);
    let tie = decimals.as_bytes().get(precision) == Some(&b'5')
        && decimals
            .bytes()
            .skip(precision + 1)
            .all(|digit| digit == b'0');
    let nudged = if tie {
        magnitude + 10f64.powi(-(precision as i32) - 3)
    } else {
        magnitude
    };
    let body = format!("{nudged:.precision$}");
    if value < 0.0 {
        format!("-{body}")
    } else {
        body
    }
}

fn srgb_to_linear(channel: f64) -> f64 {
    let c = channel / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(channel: f64, round: bool) -> f64 {
    let c = if channel <= 0.0031308 {
        channel * 12.92
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    };
    let srgb = clamp(c, 0.0, 1.0) * 255.0;
    if round { js_round(srgb) } else { srgb }
}

pub fn rgb_to_oklch(color: Rgb) -> Oklch {
    let r = srgb_to_linear(color.r);
    let g = srgb_to_linear(color.g);
    let b = srgb_to_linear(color.b);
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let lightness = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
    let a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
    let bb = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
    let hue = bb.atan2(a) * 180.0 / PI;
    Oklch {
        l: lightness,
        c: a.hypot(bb),
        h: (hue + 360.0) % 360.0,
    }
}

fn oklch_to_linear(Oklch { l, c, h }: Oklch) -> Rgb {
    let radians = h * PI / 180.0;
    let a = c * radians.cos();
    let b = c * radians.sin();
    let lms = [
        (l + 0.3963377774 * a + 0.2158037573 * b).powi(3),
        (l - 0.1055613458 * a - 0.0638541728 * b).powi(3),
        (l - 0.0894841775 * a - 1.291485548 * b).powi(3),
    ];
    Rgb {
        r: 4.0767416621 * lms[0] - 3.3077115913 * lms[1] + 0.2309699292 * lms[2],
        g: -1.2684380046 * lms[0] + 2.6097574011 * lms[1] - 0.3413193965 * lms[2],
        b: -0.0041960863 * lms[0] - 0.7034186147 * lms[1] + 1.707614701 * lms[2],
    }
}

fn in_gamut(color: Oklch) -> bool {
    let linear = oklch_to_linear(color);
    [linear.r, linear.g, linear.b]
        .iter()
        .all(|channel| (-0.0001..=1.0001).contains(channel))
}

/// Lowers the chroma until the color fits sRGB, keeping its lightness and hue.
pub fn to_srgb_gamut(color: Oklch) -> Oklch {
    if in_gamut(color) {
        return color;
    }
    let mut low = 0.0;
    let mut high = color.c;
    for _ in 0..24 {
        let mid = (low + high) / 2.0;
        if in_gamut(Oklch { c: mid, ..color }) {
            low = mid;
        } else {
            high = mid;
        }
    }
    Oklch { c: low, ..color }
}

pub fn oklch_to_rgb(color: Oklch, round: bool) -> Rgb {
    let linear = oklch_to_linear(to_srgb_gamut(color));
    Rgb {
        r: linear_to_srgb(linear.r, round),
        g: linear_to_srgb(linear.g, round),
        b: linear_to_srgb(linear.b, round),
    }
}

fn format_number(value: f64, precision: usize) -> String {
    let rounded = if value.abs() < 10f64.powi(-(precision as i32)) / 2.0 {
        0.0
    } else {
        value
    };
    let fixed = to_fixed(rounded, precision);
    fixed
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

pub fn format_oklch(color: Oklch, alpha: f64) -> String {
    let hue = if color.c < 0.0005 {
        0.0
    } else {
        ((color.h % 360.0) + 360.0) % 360.0
    };
    let body = format!(
        "{} {} {}",
        format_number(color.l, 4),
        format_number(color.c, 4),
        format_number(hue, 3)
    );
    if alpha < 1.0 {
        format!("oklch({body} / {})", format_number(alpha, 3))
    } else {
        format!("oklch({body})")
    }
}

fn parse_hex(hex: &str) -> Option<(Rgb, f64)> {
    if !matches!(hex.len(), 3 | 4 | 6 | 8) || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let short = hex.len() <= 4;
    let channel = |index: usize| {
        let digits = if short {
            hex[index..=index].repeat(2)
        } else {
            hex[index * 2..index * 2 + 2].to_string()
        };
        f64::from(u8::from_str_radix(&digits, 16).unwrap_or(0))
    };
    let has_alpha = hex.len() == 4 || hex.len() == 8;
    Some((
        Rgb::new(channel(0), channel(1), channel(2)),
        if has_alpha { channel(3) / 255.0 } else { 1.0 },
    ))
}

/// `Number.parseFloat`: the longest number at the start of `text`.
pub(crate) fn parse_float_prefix(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    let mut end = 0;
    if matches!(bytes.first(), Some(b'-' | b'+')) {
        end = 1;
    }
    let digits_start = end;
    while bytes.get(end).is_some_and(u8::is_ascii_digit) {
        end += 1;
    }
    let mut has_digits = end > digits_start;
    if bytes.get(end) == Some(&b'.') {
        let mut fraction = end + 1;
        while bytes.get(fraction).is_some_and(u8::is_ascii_digit) {
            fraction += 1;
        }
        if fraction > end + 1 || has_digits {
            has_digits = has_digits || fraction > end + 1;
            end = fraction;
        }
    }
    if !has_digits {
        return None;
    }
    if matches!(bytes.get(end), Some(b'e' | b'E')) {
        let mut exponent = end + 1;
        if matches!(bytes.get(exponent), Some(b'-' | b'+')) {
            exponent += 1;
        }
        let exponent_digits = exponent;
        while bytes.get(exponent).is_some_and(u8::is_ascii_digit) {
            exponent += 1;
        }
        if exponent > exponent_digits {
            end = exponent;
        }
    }
    text[..end].trim_end_matches('.').parse().ok()
}

/// `/^-?[\d.]+(?:e-?\d+)?%?$/i`
fn looks_numeric(part: &str) -> bool {
    let body = part.strip_suffix('%').unwrap_or(part);
    let body = body.strip_prefix('-').unwrap_or(body);
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(index) => (&body[..index], Some(&body[index + 1..])),
        None => (body, None),
    };
    let mantissa_ok =
        !mantissa.is_empty() && mantissa.bytes().all(|b| b.is_ascii_digit() || b == b'.');
    let exponent_ok = exponent.is_none_or(|exponent| {
        let digits = exponent.strip_prefix('-').unwrap_or(exponent);
        !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
    });
    mantissa_ok && exponent_ok
}

fn parse_number(part: &str, percent_scale: f64) -> Option<f64> {
    if part == "none" {
        return Some(0.0);
    }
    let value = parse_float_prefix(part)?;
    if !value.is_finite() || !looks_numeric(part) {
        return None;
    }
    Some(if part.ends_with('%') {
        value / 100.0 * percent_scale
    } else {
        value
    })
}

fn split_arguments(body: &str) -> (Vec<&str>, Option<&str>) {
    let mut parts = body.split('/');
    let channel_part = parts.next().unwrap_or("");
    let alpha_part = parts.next();
    let mut channels: Vec<&str> = channel_part
        .trim()
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|part| !part.is_empty())
        .collect();
    if let Some(alpha) = alpha_part {
        return (channels, Some(alpha.trim()));
    }
    if channels.len() == 4 {
        let alpha = channels.pop();
        return (channels, alpha);
    }
    (channels, None)
}

fn parse_alpha(part: Option<&str>) -> Option<f64> {
    match part {
        None => Some(1.0),
        Some(part) => parse_number(part, 1.0).map(|alpha| clamp(alpha, 0.0, 1.0)),
    }
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> Rgb {
    let k = |n: f64| (n + h / 30.0) % 12.0;
    let a = s * l.min(1.0 - l);
    let f = |n: f64| l - a * (-1.0f64).max((k(n) - 3.0).min((9.0 - k(n)).min(1.0)));
    Rgb::new(f(0.0) * 255.0, f(8.0) * 255.0, f(4.0) * 255.0)
}

enum Space {
    Rgb(Rgb),
    Oklch(Oklch),
}

fn parse_space(value: &str) -> Option<(Space, f64)> {
    let input = value.trim().to_lowercase();
    if let Some(hex) = input.strip_prefix('#') {
        let (rgb, alpha) = parse_hex(hex)?;
        return Some((Space::Rgb(rgb), alpha));
    }
    let open = input.find('(')?;
    let name = &input[..open];
    if !matches!(name, "rgb" | "rgba" | "hsl" | "hsla" | "oklch") {
        return None;
    }
    let body = input[open + 1..].strip_suffix(')')?;
    if body.contains(['(', ')']) {
        return None;
    }
    let (channels, alpha_part) = split_arguments(body);
    if channels.len() != 3 {
        return None;
    }
    let alpha = parse_alpha(alpha_part)?;
    let hue = |part: &str| parse_number(part.strip_suffix("deg").unwrap_or(part), 360.0);
    if name == "oklch" {
        let l = parse_number(channels[0], 1.0)?;
        let c = parse_number(channels[1], 0.4)?;
        let h = hue(channels[2])?;
        let color = Oklch {
            l: clamp(l, 0.0, 1.0),
            c: c.max(0.0),
            h: ((h % 360.0) + 360.0) % 360.0,
        };
        return Some((Space::Oklch(color), alpha));
    }
    if name.starts_with("rgb") {
        let r = parse_number(channels[0], 255.0)?;
        let g = parse_number(channels[1], 255.0)?;
        let b = parse_number(channels[2], 255.0)?;
        let rgb = Rgb::new(
            clamp(r, 0.0, 255.0),
            clamp(g, 0.0, 255.0),
            clamp(b, 0.0, 255.0),
        );
        return Some((Space::Rgb(rgb), alpha));
    }
    let h = hue(channels[0])?;
    let s = parse_number(channels[1], 1.0)?;
    let l = parse_number(channels[2], 1.0)?;
    let rgb = hsl_to_rgb(
        ((h % 360.0) + 360.0) % 360.0,
        clamp(s, 0.0, 1.0),
        clamp(l, 0.0, 1.0),
    );
    Some((Space::Rgb(rgb), alpha))
}

/// Reads hex, `rgb()`, `hsl()` and `oklch()` colors; anything else is `None`.
pub fn parse_color(value: &str) -> Option<ParsedColor> {
    let (space, alpha) = parse_space(value)?;
    let color = match space {
        Space::Rgb(rgb) => rgb_to_oklch(rgb),
        Space::Oklch(oklch) => oklch,
    };
    Some(ParsedColor { color, alpha })
}

/// `parse_color` for a JSON value, which is no color unless it's a string.
pub fn parse_color_value(value: &serde_json::Value) -> Option<ParsedColor> {
    value.as_str().and_then(parse_color)
}

/// A stored color as sRGB channels and alpha, each in `0..=1`.
pub fn css_to_rgba(value: &str) -> Option<[f32; 4]> {
    let (space, alpha) = parse_space(value)?;
    let rgb = match space {
        Space::Rgb(rgb) => rgb,
        Space::Oklch(oklch) => oklch_to_rgb(oklch, false),
    };
    Some([
        (rgb.r / 255.0) as f32,
        (rgb.g / 255.0) as f32,
        (rgb.b / 255.0) as f32,
        alpha as f32,
    ])
}

pub fn is_color(value: &str) -> bool {
    parse_color(value).is_some()
}

/// The color as `oklch()`, the form themes store; `None` when it isn't a color.
pub fn canonical_color(value: &str) -> Option<String> {
    parse_color(value).map(|parsed| format_oklch(parsed.color, parsed.alpha))
}

pub fn rgb_to_hex(color: Rgb) -> String {
    let channel = |value: f64| js_round(clamp(value, 0.0, 255.0)) as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(color.r),
        channel(color.g),
        channel(color.b)
    )
}

pub fn color_to_hex(value: &str) -> Option<String> {
    let parsed = parse_color(value)?;
    let hex = rgb_to_hex(oklch_to_rgb(parsed.color, true));
    if parsed.alpha >= 1.0 {
        return Some(hex);
    }
    Some(format!("{hex}{:02x}", js_round(parsed.alpha * 255.0) as u8))
}

pub fn color_to_rgb(value: &str, fallback: Rgb) -> Rgb {
    parse_color(value).map_or(fallback, |parsed| oklch_to_rgb(parsed.color, true))
}

pub fn relative_luminance(color: Rgb) -> f64 {
    0.2126 * srgb_to_linear(color.r)
        + 0.7152 * srgb_to_linear(color.g)
        + 0.0722 * srgb_to_linear(color.b)
}

pub fn is_dark_rgb(color: Rgb) -> bool {
    relative_luminance(color) < DARK_LUMINANCE
}

pub fn contrast_ratio(first: Rgb, second: Rgb) -> f64 {
    let a = relative_luminance(first);
    let b = relative_luminance(second);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

pub fn mix_rgb(base: Rgb, overlay: Rgb, amount: f64) -> Rgb {
    Rgb {
        r: base.r + (overlay.r - base.r) * amount,
        g: base.g + (overlay.g - base.g) * amount,
        b: base.b + (overlay.b - base.b) * amount,
    }
}

pub fn rgb_to_color(color: Rgb, alpha: f64) -> String {
    format_oklch(rgb_to_oklch(color), alpha)
}

pub fn oklch_to_color(color: Oklch, alpha: f64) -> String {
    format_oklch(to_srgb_gamut(color), alpha)
}

/// Moves only the lightness until `base` reaches `min_contrast` on every surface.
pub fn solve_lightness(
    base: Oklch,
    against: &[Rgb],
    min_contrast: f64,
    direction: LightnessDirection,
) -> Oklch {
    let contrast_of = |color: Oklch| {
        let rgb = oklch_to_rgb(color, true);
        against
            .iter()
            .map(|surface| contrast_ratio(rgb, *surface))
            .fold(f64::INFINITY, f64::min)
    };
    if contrast_of(base) >= min_contrast {
        return base;
    }
    let lighter = direction == LightnessDirection::Lighter;
    let mut low = if lighter { base.l } else { 0.0 };
    let mut high = if lighter { 1.0 } else { base.l };
    for _ in 0..18 {
        let mid = (low + high) / 2.0;
        let passes = contrast_of(Oklch { l: mid, ..base }) >= min_contrast;
        if lighter == passes {
            high = mid;
        } else {
            low = mid;
        }
    }
    Oklch {
        l: if lighter { high } else { low },
        ..base
    }
}

/// White or black, whichever reads better on `background`.
pub fn readable_foreground(background: Rgb) -> Rgb {
    if contrast_ratio(background, WHITE) >= contrast_ratio(background, BLACK) {
        WHITE
    } else {
        BLACK
    }
}

/// `foreground` softened toward `background` as far as `minimum_ratio` allows.
pub fn softened_text(background: Rgb, foreground: Rgb, minimum_ratio: f64) -> Rgb {
    let mut readable = foreground;
    let mut low = 0.0;
    let mut high = 1.0;
    for _ in 0..12 {
        let amount = (low + high) / 2.0;
        let candidate = mix_rgb(foreground, background, amount);
        if contrast_ratio(candidate, background) >= minimum_ratio {
            readable = candidate;
            low = amount;
        } else {
            high = amount;
        }
    }
    readable
}

/// `color` with its lightness moved until it reads on `surface`, keeping its hue.
pub fn readable_on(color: &str, surface: &str, min_contrast: f64) -> String {
    let (Some(parsed_color), Some(parsed_surface)) = (parse_color(color), parse_color(surface))
    else {
        return color.to_string();
    };
    let surface_rgb = oklch_to_rgb(parsed_surface.color, true);
    if contrast_ratio(oklch_to_rgb(parsed_color.color, true), surface_rgb) >= min_contrast {
        return color.to_string();
    }
    let direction = if is_dark_rgb(surface_rgb) {
        LightnessDirection::Lighter
    } else {
        LightnessDirection::Darker
    };
    let solved = solve_lightness(parsed_color.color, &[surface_rgb], min_contrast, direction);
    rgb_to_color(oklch_to_rgb(solved, true), 1.0)
}

pub fn rgb_to_hsv(Rgb { r, g, b }: Rgb) -> Hsv {
    let (red, green, blue) = (r / 255.0, g / 255.0, b / 255.0);
    let max = red.max(green).max(blue);
    let delta = max - red.min(green).min(blue);
    let mut h = 0.0;
    if delta > 0.0 {
        h = if max == red {
            ((green - blue) / delta) % 6.0
        } else if max == green {
            (blue - red) / delta + 2.0
        } else {
            (red - green) / delta + 4.0
        };
    }
    Hsv {
        h: (h * 60.0 + 360.0) % 360.0,
        s: if max == 0.0 { 0.0 } else { delta / max },
        v: max,
    }
}

pub fn hsv_to_rgb(Hsv { h, s, v }: Hsv) -> Rgb {
    let f = |n: f64| {
        let k = (n + h / 60.0) % 6.0;
        (v - v * s * 0.0f64.max(k.min(4.0 - k).min(1.0))) * 255.0
    };
    Rgb::new(f(5.0), f(3.0), f(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(value: &str) -> String {
        color_to_hex(value).unwrap()
    }

    #[test]
    fn reads_hex_in_every_length() {
        assert_eq!(hex("#fff"), "#ffffff");
        assert_eq!(hex("#ff000080"), "#ff000080");
        assert_eq!(hex("#1b4ed8"), "#1b4ed8");
    }

    #[test]
    fn reads_rgb_hsl_and_oklch() {
        assert_eq!(hex("rgb(255 0 0)"), "#ff0000");
        assert_eq!(hex("rgba(0, 0, 255, 0.5)"), "#0000ff80");
        assert_eq!(hex("hsl(120 100% 50%)"), "#00ff00");
        assert_eq!(hex("oklch(1 0 0)"), "#ffffff");
        let alpha = parse_color("oklch(1 0 0 / 10%)").unwrap().alpha;
        assert!((alpha - 0.1).abs() < 1e-9);
    }

    #[test]
    fn rejects_what_is_not_a_literal_color() {
        assert!(!is_color("red"));
        assert!(!is_color("var(--primary)"));
        assert!(!is_color("#12"));
        assert!(!is_color("rgb(1 2)"));
        assert!(parse_color_value(&serde_json::json!(12)).is_none());
    }

    #[test]
    fn stores_colors_as_oklch_and_survives_a_round_trip() {
        let canonical = canonical_color("#1b4ed8").unwrap();
        assert!(canonical.starts_with("oklch("));
        assert_eq!(hex(&canonical), "#1b4ed8");
    }

    #[test]
    fn canonical_keeps_the_alpha() {
        assert!(canonical_color("#00000080").unwrap().ends_with("/ 0.502)"));
    }

    #[test]
    fn readable_on_leaves_a_readable_color_alone() {
        assert_eq!(readable_on("#000000", "#ffffff", 4.5), "#000000");
    }

    #[test]
    fn readable_on_darkens_a_color_that_is_too_light_on_a_light_surface() {
        let fixed = readable_on("#ffee88", "#ffffff", 4.5);
        assert!(parse_color(&fixed).is_some());
        let hex = hex(&fixed);
        let channel = |index: usize| {
            f64::from(u8::from_str_radix(&hex[1 + index * 2..3 + index * 2], 16).unwrap())
        };
        let rgb = Rgb::new(channel(0), channel(1), channel(2));
        assert!(contrast_ratio(rgb, WHITE) >= 4.5);
    }

    #[test]
    fn black_on_white_is_21() {
        assert!((contrast_ratio(BLACK, WHITE) - 21.0).abs() < 1e-9);
    }

    #[test]
    fn hsv_round_trips_through_rgb() {
        for hex in ["#1b4ed8", "#ff8800", "#000000", "#ffffff", "#7a7a7a"] {
            let rgb = color_to_rgb(hex, BLACK);
            assert_eq!(rgb_to_hex(hsv_to_rgb(rgb_to_hsv(rgb))), hex);
        }
    }

    #[test]
    fn to_fixed_rounds_ties_like_javascript() {
        assert_eq!(to_fixed(0.25, 1), "0.3");
        assert_eq!(to_fixed(0.03125, 4), "0.0313");
        assert_eq!(to_fixed(-0.125, 2), "-0.13");
        assert_eq!(to_fixed(-0.0, 4), "0.0000");
        assert_eq!(to_fixed(1.005, 2), "1.00");
    }

    #[test]
    fn parses_numbers_like_parse_float() {
        assert_eq!(parse_float_prefix("1.5%"), Some(1.5));
        assert_eq!(parse_float_prefix(".5"), Some(0.5));
        assert_eq!(parse_float_prefix("1."), Some(1.0));
        assert_eq!(parse_float_prefix("1.2.3"), Some(1.2));
        assert_eq!(parse_float_prefix("2e-3"), Some(0.002));
        assert_eq!(parse_float_prefix("."), None);
        assert_eq!(parse_float_prefix("none"), None);
    }

    #[test]
    fn css_to_rgba_reads_every_syntax() {
        assert_eq!(
            css_to_rgba("#ff000080"),
            Some([1.0, 0.0, 0.0, 128.0 / 255.0])
        );
        assert_eq!(
            css_to_rgba("rgb(0 0 255 / 50%)"),
            Some([0.0, 0.0, 1.0, 0.5])
        );
        let [r, g, b, a] = css_to_rgba("hsl(120deg 100% 50%)").unwrap();
        assert!(r.abs() < 1e-6 && (g - 1.0).abs() < 1e-6 && b.abs() < 1e-6 && a == 1.0);
        let [r, g, b, a] = css_to_rgba("oklch(1 0 0 / 0.1)").unwrap();
        assert!(r > 0.999 && g > 0.999 && b > 0.999);
        assert!((a - 0.1).abs() < 1e-6);
        assert_eq!(css_to_rgba("transparent"), None);
    }
}
