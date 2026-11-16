//! What the app looks like: the theme, its light and dark halves, and the fonts. Kept in
//! `appearance.json` in hoshi's home, in the shape the web keeps in its storage.

use crate::themes::appearance::{
    Appearance, CODE_SIZE, COMPOSER_SIZE, INTERFACE_SIZE, TERMINAL_SIZE, clamp_size,
    normalize_appearance, resolve_appearance,
};
use crate::themes::color::css_to_rgba;
use crate::themes::model::{ThemeAppearance, ThemeColors, resolve_theme_for, role_variable};
use gpui_kit::*;
use hoshi_ui::theme::{self, Colors, Fonts, ThemeMode};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub struct Look {
    pub appearance: Appearance,
    path: Option<PathBuf>,
    /// Colors the theme editor shows on the whole app while it is open.
    pub preview: Option<(ThemeAppearance, ThemeColors)>,
}

impl Global for Look {}

fn file(home: &Path) -> PathBuf {
    home.join("appearance.json")
}

/// Reads the saved appearance; the first spike saved only `colorScheme`.
fn parse(text: &str) -> Appearance {
    let mut value: Value = serde_json::from_str(text).unwrap_or_default();
    if let Some(object) = value.as_object_mut()
        && !object.contains_key("preference")
        && let Some(scheme) = object.get("colorScheme").cloned()
    {
        object.insert("preference".into(), scheme);
    }
    normalize_appearance(&value)
}

pub fn load(home: &Path) -> Appearance {
    std::fs::read_to_string(file(home))
        .map(|text| parse(&text))
        .unwrap_or_else(|_| normalize_appearance(&Value::Null))
}

fn save(path: &Path, appearance: &Appearance) -> std::io::Result<()> {
    let text =
        serde_json::to_string_pretty(&appearance.to_json()).map_err(std::io::Error::other)?;
    std::fs::write(path, text)
}

/// Shows the saved appearance in `window`, and follows the system's from now on.
pub fn init(home: Option<&Path>, window: &mut Window, cx: &mut App) {
    let appearance = home
        .map(load)
        .unwrap_or_else(|| normalize_appearance(&Value::Null));
    cx.set_global(Look {
        appearance,
        path: home.map(file),
        preview: None,
    });
    theme::set_painter(paint, cx);
    theme::set_fonts(fonts(&cx.global::<Look>().appearance, cx), cx);
    cx.set_text_rendering_mode(text_rendering(&cx.global::<Look>().appearance));
    theme::follow_system(window, cx);
}

pub fn appearance(cx: &App) -> &Appearance {
    &cx.global::<Look>().appearance
}

/// Changes the appearance, shows it and saves it.
pub fn update(cx: &mut App, change: impl FnOnce(&mut Appearance)) {
    let look = cx.global_mut::<Look>();
    let before = look.appearance.clone();
    change(&mut look.appearance);
    if look.appearance == before {
        return;
    }
    let saved = (look.path.clone(), look.appearance.clone());
    let fonts = fonts(&saved.1, cx);
    theme::set_fonts(fonts, cx);
    cx.set_text_rendering_mode(text_rendering(&saved.1));
    theme::refresh(cx);
    if let (Some(path), appearance) = saved {
        cx.background_spawn(async move { save(&path, &appearance) })
            .detach_and_log_err(cx);
    }
}

/// Shows `preview` on the whole app instead of the chosen theme; `None` goes back to it.
pub fn set_preview(preview: Option<(ThemeAppearance, ThemeColors)>, cx: &mut App) {
    cx.global_mut::<Look>().preview = preview;
    theme::refresh(cx);
}

/// Whether the app shows light or dark now, after the mode and the themes' halves.
pub fn resolved(window: &Window, cx: &App) -> ThemeAppearance {
    let look = cx.global::<Look>();
    if let Some((appearance, _)) = &look.preview {
        return *appearance;
    }
    let system_dark = ThemeMode::from(window.appearance()).is_dark();
    resolve_appearance(&look.appearance, system_dark)
}

fn paint(system_dark: bool, cx: &App) -> (ThemeMode, Colors) {
    let look = cx.global::<Look>();
    match &look.preview {
        Some((appearance, colors)) => to_colors(*appearance, colors),
        None => {
            let appearance = resolve_appearance(&look.appearance, system_dark);
            let resolved = resolve_theme_for(
                &look.appearance.selection(),
                appearance,
                &look.appearance.custom_themes,
            );
            to_colors(appearance, resolved.colors)
        }
    }
}

pub fn mode(appearance: ThemeAppearance) -> ThemeMode {
    match appearance {
        ThemeAppearance::Light => ThemeMode::Light,
        ThemeAppearance::Dark => ThemeMode::Dark,
    }
}

/// A GPUI color from a stored CSS color.
pub fn color(value: &str) -> Option<Hsla> {
    let [r, g, b, a] = css_to_rgba(value)?;
    Some(Rgba { r, g, b, a }.into())
}

/// `globals.css`'s colors for the appearance, with the theme's on top.
pub fn to_colors(appearance: ThemeAppearance, colors: &ThemeColors) -> (ThemeMode, Colors) {
    let mode = mode(appearance);
    let mut painted = theme::colors(mode);
    for (role, value) in colors.iter() {
        if let Some(color) = color(value) {
            painted.set(&role_variable(role), color);
        }
    }
    (mode, painted)
}

/// The family a font setting names: `None` keeps the default.
pub fn family(value: &str, monospace: bool, cx: &App) -> Option<SharedString> {
    match value {
        "geist" | "jetbrains" | "interface" => None,
        "inter" => Some(theme::INTER.into()),
        "cascadia" => Some(first_installed(&["Cascadia Mono", "Cascadia Code"], cx)?),
        "system" if !monospace => Some(".SystemUIFont".into()),
        "system" | "system-mono" => Some(first_installed(
            &[
                "SF Mono",
                "Menlo",
                "Consolas",
                "DejaVu Sans Mono",
                "Liberation Mono",
            ],
            cx,
        )?),
        other => Some(SharedString::from(other.to_owned())),
    }
}

fn first_installed(families: &[&str], cx: &App) -> Option<SharedString> {
    let installed = cx.text_system().all_font_names();
    families
        .iter()
        .find(|family| installed.iter().any(|name| name == *family))
        .map(|family| SharedString::from(*family))
}

/// Font smoothing draws text in grayscale, thinner than the subpixel colors it uses off.
fn text_rendering(appearance: &Appearance) -> TextRenderingMode {
    if appearance.font_smoothing {
        TextRenderingMode::Grayscale
    } else {
        TextRenderingMode::Subpixel
    }
}

fn fonts(appearance: &Appearance, cx: &App) -> Fonts {
    let sans = family(&appearance.interface_font, false, cx);
    let composer = appearance.separate_fonts.then(|| {
        let family = match appearance.composer_font.as_str() {
            "interface" => sans.clone(),
            "jetbrains" | "cascadia" | "system-mono" => {
                family(&appearance.composer_font, true, cx).or_else(|| theme::default_mono(cx))
            }
            other => family(other, false, cx),
        };
        (
            family.unwrap_or_else(|| theme::SANS.into()),
            px(clamp_size(appearance.composer_size as f64, COMPOSER_SIZE) as f32),
        )
    });
    let terminal = appearance.separate_fonts.then(|| {
        (
            family(&appearance.terminal_font, true, cx)
                .or_else(|| theme::default_mono(cx))
                .unwrap_or_else(|| theme::SANS.into()),
            px(clamp_size(appearance.terminal_size as f64, TERMINAL_SIZE) as f32),
        )
    });
    Fonts {
        sans,
        mono: family(&appearance.code_font, true, cx),
        size: px(clamp_size(appearance.interface_size as f64, INTERFACE_SIZE) as f32),
        mono_size: px(clamp_size(appearance.code_size as f64, CODE_SIZE) as f32),
        composer,
        terminal,
        word_wrap: appearance.word_wrap,
    }
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::themes::appearance::ThemePreference;
    use gpui_kit::{
        AppContext as _, Bounds, EmptyView, Point, TestAppContext, TextRenderingMode, WindowBounds,
        WindowOptions, px, size,
    };

    #[gpui_kit::test]
    fn font_smoothing_draws_text_in_grayscale_and_subpixel_without_it(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(400.), px(300.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                super::init(None, window, cx);
                cx.new(|_| EmptyView)
            })
            .unwrap();
            assert_eq!(cx.text_rendering_mode(), TextRenderingMode::Grayscale);
            super::update(cx, |look| look.font_smoothing = false);
            assert_eq!(cx.text_rendering_mode(), TextRenderingMode::Subpixel);
        });
    }

    #[test]
    fn the_first_spike_file_keeps_its_color_scheme() {
        assert_eq!(
            parse(r#"{"colorScheme":"light"}"#).preference,
            ThemePreference::Light
        );
        assert_eq!(parse("not json").preference, ThemePreference::System);
    }
}
