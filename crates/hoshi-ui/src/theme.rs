//! The colors come from `packages/ui/src/styles/globals.css`, the same tokens the web UI uses.

use gpui_kit::component::theme::Theme;
pub use gpui_kit::component::theme::ThemeMode;
use gpui_kit::*;
use std::rc::Rc;

const GLOBALS_CSS: &str = include_str!("../../../packages/ui/src/styles/globals.css");

pub const SANS: &str = "Geist";
/// The other interface font hoshi ships, as the web does.
pub const INTER: &str = "Inter";
/// `--stack-jetbrains` in `globals.css`, whose first face hoshi doesn't ship.
const MONO_STACK: &[&str] = &["JetBrains Mono", "Cascadia Mono", "Consolas", "Menlo"];

const FONTS: &[&[u8]] = &[
    include_bytes!("../assets/fonts/Geist-Regular.ttf"),
    include_bytes!("../assets/fonts/Geist-Medium.ttf"),
    include_bytes!("../assets/fonts/Geist-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Geist-Bold.ttf"),
    include_bytes!("../assets/fonts/Geist-Italic.ttf"),
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/Inter-Bold.ttf"),
    include_bytes!("../assets/fonts/Inter-Italic.ttf"),
];

pub fn load_fonts(cx: &mut App) {
    let fonts = FONTS.iter().map(|font| (*font).into()).collect();
    if let Err(error) = cx.text_system().add_fonts(fonts) {
        eprintln!("hoshi-ui: could not load the bundled fonts: {error}");
    }
}

macro_rules! tokens {
    ($($field:ident),* $(,)?) => {
        #[derive(Clone, Debug)]
        #[allow(dead_code)]
        pub struct Colors {
            $(pub $field: Hsla,)*
        }

        impl Colors {
            /// Sets the token named as in `globals.css` without `--`, as `sidebar-border`;
            /// `false` when there is none.
            pub fn set(&mut self, token: &str, color: Hsla) -> bool {
                $(if token == stringify!($field).replace('_', "-") {
                    self.$field = color;
                    return true;
                })*
                false
            }

            fn from_block(block: &str) -> Result<Self, String> {
                Ok(Self {
                    $($field: token(block, &stringify!($field).replace('_', "-"))?,)*
                })
            }
        }
    };
}

tokens! {
    background, foreground, card, card_foreground, popover, popover_foreground,
    primary, primary_foreground, secondary, secondary_foreground, muted, muted_foreground,
    accent, accent_foreground, destructive, destructive_foreground, border, input, ring,
    sidebar, sidebar_foreground, sidebar_primary, sidebar_primary_foreground,
    sidebar_accent, sidebar_accent_foreground, sidebar_border, sidebar_ring,
    header, header_star, header_glow, status_working, status_waiting, draft, draft_surface,
    warning, status_error, status_done,
    provider_claude, provider_copilot, provider_codex, favorite, link, send, pr_open, pr_merged,
    diff_added, diff_removed,
    backdrop, scrollbar, scrollbar_hover, window_close, window_close_foreground, terminal_selection,
    file_blue, file_cyan, file_teal, file_green, file_yellow, file_orange, file_red, file_pink,
    file_purple, file_magenta, file_gray,
}

pub struct HoshiTheme {
    pub colors: Colors,
}

impl Global for HoshiTheme {}

pub trait ActiveTheme {
    fn hoshi(&self) -> &Colors;
}

impl ActiveTheme for App {
    fn hoshi(&self) -> &Colors {
        &self.global::<HoshiTheme>().colors
    }
}

type Painter = Rc<dyn Fn(bool, &App) -> (ThemeMode, Colors)>;

/// Decides the mode and the colors the app shows from whether the system is dark.
struct ThemePainter(Painter);

impl Global for ThemePainter {}

/// Lets the app pick the colors: `paint` gets whether the system is dark and returns the mode
/// and colors to show. Without one, the app follows the system with `globals.css`'s colors.
pub fn set_painter(paint: impl Fn(bool, &App) -> (ThemeMode, Colors) + 'static, cx: &mut App) {
    cx.set_global(ThemePainter(Rc::new(paint)));
}

/// Whether the system was dark when the window was last asked.
struct SystemDark(bool);

impl Global for SystemDark {}

/// Shows what the painter picks now.
pub fn repaint(window: &Window, cx: &mut App) {
    let dark = ThemeMode::from(window.appearance()).is_dark();
    cx.set_global(SystemDark(dark));
    paint(dark, cx);
}

/// Shows what the painter picks now, for the system's appearance last seen.
pub fn refresh(cx: &mut App) {
    let dark = cx.try_global::<SystemDark>().is_some_and(|dark| dark.0);
    paint(dark, cx);
}

fn paint(dark: bool, cx: &mut App) {
    let (mode, colors) = match cx.try_global::<ThemePainter>() {
        Some(painter) => {
            let paint = painter.0.clone();
            paint(dark, cx)
        }
        None => {
            let mode = if dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            };
            (mode, colors(mode))
        }
    };
    apply_colors(mode, colors, cx);
}

/// Shows what the painter picks, now and when the system's appearance changes.
pub fn follow_system(window: &mut Window, cx: &mut App) {
    repaint(window, cx);
    window
        .observe_window_appearance(|window, cx| repaint(window, cx))
        .detach();
}

/// `globals.css`'s colors for one mode.
pub fn colors(mode: ThemeMode) -> Colors {
    parse(mode).expect("globals.css defines every token")
}

/// The fonts and sizes text uses. `None` families keep the defaults: Geist, and the first
/// monospace font installed.
#[derive(Clone, Debug, PartialEq)]
pub struct Fonts {
    pub sans: Option<SharedString>,
    pub mono: Option<SharedString>,
    /// What 1rem is: the interface's text size.
    pub size: Pixels,
    pub mono_size: Pixels,
    /// The prompt box's own font and size, when it doesn't follow the interface.
    pub composer: Option<(SharedString, Pixels)>,
    /// The terminal's own font and size, when it doesn't follow the code's.
    pub terminal: Option<(SharedString, Pixels)>,
    /// Long lines in code and diffs wrap; off, they scroll sideways.
    pub word_wrap: bool,
}

impl Default for Fonts {
    fn default() -> Self {
        Self {
            sans: None,
            mono: None,
            size: px(16.),
            mono_size: px(13.),
            composer: None,
            terminal: None,
            word_wrap: true,
        }
    }
}

impl Global for Fonts {}

pub fn fonts(cx: &App) -> Fonts {
    cx.try_global::<Fonts>().cloned().unwrap_or_default()
}

/// The terminal's family and size: its own, or the code's.
pub fn terminal_font(cx: &App) -> (SharedString, Pixels) {
    cx.try_global::<Fonts>()
        .and_then(|fonts| fonts.terminal.clone())
        .unwrap_or_else(|| {
            let theme = Theme::global(cx);
            (theme.mono_font_family.clone(), theme.mono_font_size)
        })
}

/// Uses `fonts` from now on.
pub fn set_fonts(fonts: Fonts, cx: &mut App) {
    cx.set_global(fonts);
    apply_fonts(cx);
    cx.refresh_windows();
}

/// The installed font families, sorted, for a font picker.
pub fn installed_families(cx: &App) -> Vec<SharedString> {
    let mut names: Vec<SharedString> = cx
        .text_system()
        .all_font_names()
        .into_iter()
        .filter(|name| !name.starts_with('.'))
        .map(SharedString::from)
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup();
    names
}

/// The monospace family the default stack resolves to here.
pub fn default_mono(cx: &App) -> Option<SharedString> {
    let installed = cx.text_system().all_font_names();
    MONO_STACK
        .iter()
        .find(|family| installed.iter().any(|name| name == *family))
        .map(|family| SharedString::from(*family))
}

fn apply_fonts(cx: &mut App) {
    let fonts = fonts(cx);
    let mono = fonts.mono.clone().or_else(|| default_mono(cx));
    Theme::update(cx, |theme| {
        theme.font_family = fonts.sans.clone().unwrap_or_else(|| SANS.into());
        if let Some(mono) = mono {
            theme.mono_font_family = mono;
        }
        theme.font_size = fonts.size;
        theme.mono_font_size = fonts.mono_size;
    });
}

pub fn apply(mode: ThemeMode, cx: &mut App) {
    apply_colors(mode, colors(mode), cx);
}

/// Shows `colors`, with `mode` for the components that follow light or dark on their own.
pub fn apply_colors(mode: ThemeMode, colors: Colors, cx: &mut App) {
    Theme::change(mode, None, cx);
    Theme::update(cx, |theme| {
        let c = &mut theme.colors;
        c.background = colors.background;
        c.foreground = colors.foreground;
        c.popover = colors.popover;
        c.popover_foreground = colors.popover_foreground;
        c.primary = colors.primary;
        c.primary_foreground = colors.primary_foreground;
        c.secondary = colors.secondary;
        c.secondary_foreground = colors.secondary_foreground;
        c.muted = colors.muted;
        c.muted_foreground = colors.muted_foreground;
        c.accent = colors.accent;
        c.accent_foreground = colors.accent_foreground;
        c.danger = colors.destructive;
        c.border = colors.border;
        c.input = colors.input;
        c.ring = colors.ring;
        c.link = colors.link;
        c.warning = colors.warning;
        c.sidebar = colors.sidebar;
        c.sidebar_foreground = colors.sidebar_foreground;
        c.sidebar_primary = colors.sidebar_primary;
        c.sidebar_primary_foreground = colors.sidebar_primary_foreground;
        c.sidebar_accent = colors.sidebar_accent;
        c.sidebar_accent_foreground = colors.sidebar_accent_foreground;
        c.sidebar_border = colors.sidebar_border;
    });
    apply_fonts(cx);
    cx.set_global(HoshiTheme { colors });
    cx.refresh_windows();
}

fn parse(mode: ThemeMode) -> Result<Colors, String> {
    let selector = match mode {
        ThemeMode::Light => ".light {",
        ThemeMode::Dark => "\n.dark {",
    };
    let start = GLOBALS_CSS
        .find(selector)
        .ok_or_else(|| format!("no `{selector}` block"))?;
    let rest = &GLOBALS_CSS[start..];
    let end = rest.find("\n}").ok_or("unclosed block")?;
    Colors::from_block(&rest[..end])
}

fn token(block: &str, name: &str) -> Result<Hsla, String> {
    let prefix = format!("--{name}:");
    let value = block
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix(&prefix))
        .ok_or_else(|| format!("--{name} is missing"))?
        .trim()
        .trim_end_matches(';')
        .trim();
    color(value).ok_or_else(|| format!("--{name}: cannot read `{value}`"))
}

fn color(value: &str) -> Option<Hsla> {
    if let Some(hex) = value.strip_prefix('#') {
        let rgb = u32::from_str_radix(hex, 16).ok()?;
        return Some(rgb_hex(rgb));
    }
    let inner = value.strip_prefix("oklch(")?.strip_suffix(')')?;
    let (lch, alpha) = match inner.split_once('/') {
        Some((lch, alpha)) => (lch, number(alpha)?),
        None => (inner, 1.0),
    };
    let mut parts = lch.split_whitespace().map(number);
    let (l, c, h) = (parts.next()??, parts.next()??, parts.next()??);
    Some(oklch(l, c, h, alpha))
}

/// A plain number, or a percentage as a fraction.
fn number(text: &str) -> Option<f32> {
    let text = text.trim();
    match text.strip_suffix('%') {
        Some(percent) => percent.parse::<f32>().ok().map(|value| value / 100.0),
        None => text.parse().ok(),
    }
}

fn rgb_hex(rgb: u32) -> Hsla {
    Rgba {
        r: ((rgb >> 16) & 0xff) as f32 / 255.0,
        g: ((rgb >> 8) & 0xff) as f32 / 255.0,
        b: (rgb & 0xff) as f32 / 255.0,
        a: 1.0,
    }
    .into()
}

fn oklch(l: f32, c: f32, h: f32, alpha: f32) -> Hsla {
    let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
    let l_ = (l + 0.396_337_8 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_35 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    let linear = [
        4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_,
        -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_,
        -0.004_196_086 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
    ];
    let [r, g, b] = linear.map(|x| {
        let x = x.clamp(0.0, 1.0);
        if x <= 0.003_130_8 {
            12.92 * x
        } else {
            1.055 * x.powf(1.0 / 2.4) - 0.055
        }
    });
    Rgba { r, g, b, a: alpha }.into()
}

/// The web's `focus-visible:ring-3 ring-ring/50`, as a shadow that keeps the layout.
pub fn focus_ring(colors: &Colors) -> Vec<gpui_kit::BoxShadow> {
    vec![gpui_kit::BoxShadow {
        color: colors.ring.opacity(0.5),
        offset: gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(0.)),
        blur_radius: gpui_kit::px(0.),
        spread_radius: gpui_kit::px(3.),
        inset: false,
    }]
}

#[cfg(test)]
mod tests {
    use super::{ThemeMode, color, parse};
    use gpui_kit::Rgba;

    #[test]
    fn a_token_is_set_by_its_css_name() {
        let mut colors = parse(ThemeMode::Light).unwrap();
        let red = gpui_kit::red();
        assert!(colors.set("sidebar-border", red));
        assert_eq!(colors.sidebar_border, red);
        assert!(!colors.set("no-such-token", red));
    }

    #[test]
    fn both_themes_define_every_token() {
        parse(ThemeMode::Light).unwrap();
        parse(ThemeMode::Dark).unwrap();
    }

    #[test]
    fn oklch_matches_srgb() {
        let white: Rgba = color("oklch(1 0 0)").unwrap().to_rgb();
        assert!((white.r - 1.0).abs() < 0.01 && (white.b - 1.0).abs() < 0.01);
        let dark: Rgba = color("oklch(0.145 0 0)").unwrap().to_rgb();
        assert!((dark.r * 255.0 - 10.0).abs() < 1.5, "{dark:?}");
        let faded = color("oklch(0.2 0 0 / 0.25)").unwrap();
        assert!((faded.a - 0.25).abs() < f32::EPSILON);
        let percent = color("oklch(1 0 0 / 10%)").unwrap();
        assert!((percent.a - 0.1).abs() < 0.001);
    }
}
