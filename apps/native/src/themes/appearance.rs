//! Theme mode, fonts and sizes (`state/appearance.ts`), stored as the web's `hoshi-theme` state.

use super::model::{
    STANDARD_THEME_ID, ThemeAppearance, ThemeDefinition, ThemeHalves, ThemeSelection, find_theme,
    parse_halves, parse_stored_themes, resolve_theme_appearance, utf16_len, valid_halves,
};
use serde_json::{Value, json};
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemePreference {
    System,
    Light,
    Dark,
}

impl ThemePreference {
    pub const ALL: [ThemePreference; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|preference| preference.as_str() == value)
    }
}

/// Fonts with a stack of their own; any other value names an installed family.
pub const INTERFACE_FONTS: [&str; 3] = ["geist", "inter", "system"];
pub const CODE_FONTS: [&str; 3] = ["jetbrains", "cascadia", "system"];
/// The prompt box follows the interface font unless it is given one of the others.
pub const COMPOSER_FONTS: [&str; 7] = [
    "interface",
    "geist",
    "inter",
    "system",
    "jetbrains",
    "cascadia",
    "system-mono",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontSlot {
    Interface,
    Composer,
    Code,
    Terminal,
}

pub fn font_presets(slot: FontSlot) -> &'static [&'static str] {
    match slot {
        FontSlot::Interface => &INTERFACE_FONTS,
        FontSlot::Composer => &COMPOSER_FONTS,
        FontSlot::Code | FontSlot::Terminal => &CODE_FONTS,
    }
}

pub fn is_font_preset(slot: FontSlot, value: &str) -> bool {
    font_presets(slot).contains(&value)
}

const MAX_FAMILY_LENGTH: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SizeRange {
    pub min: u32,
    pub max: u32,
    pub default: u32,
}

pub const INTERFACE_SIZE: SizeRange = SizeRange {
    min: 12,
    max: 20,
    default: 16,
};
pub const COMPOSER_SIZE: SizeRange = SizeRange {
    min: 12,
    max: 20,
    default: 14,
};
pub const CODE_SIZE: SizeRange = SizeRange {
    min: 10,
    max: 18,
    default: 13,
};
pub const TERMINAL_SIZE: SizeRange = SizeRange {
    min: 8,
    max: 20,
    default: 12,
};

pub fn size_options(range: SizeRange) -> Vec<u32> {
    (range.min..=range.max).collect()
}

/// Rounds to whole pixels inside `range`; anything but a finite number takes its default.
pub fn clamp_size(value: f64, range: SizeRange) -> u32 {
    if !value.is_finite() {
        return range.default;
    }
    let rounded = super::color::js_round(value);
    rounded.clamp(f64::from(range.min), f64::from(range.max)) as u32
}

#[derive(Clone, Debug, PartialEq)]
pub struct Appearance {
    pub preference: ThemePreference,
    /// The whole theme: hoshi's own, a built-in palette or a custom theme.
    pub theme: String,
    pub halves: ThemeHalves,
    pub custom_themes: Vec<ThemeDefinition>,
    /// A preset from `INTERFACE_FONTS` or the name of an installed family, as for the other fonts.
    pub interface_font: String,
    pub interface_size: u32,
    pub code_font: String,
    pub code_size: u32,
    /// Gives the prompt box and the terminal their own fonts instead of following the others.
    pub separate_fonts: bool,
    pub composer_font: String,
    pub composer_size: u32,
    pub terminal_font: String,
    pub terminal_size: u32,
    /// Wraps long lines in code blocks and diffs.
    pub word_wrap: bool,
    /// Thinner grayscale smoothing on macOS.
    pub font_smoothing: bool,
    pub lighter_text: bool,
}

pub static DEFAULT_APPEARANCE: LazyLock<Appearance> = LazyLock::new(|| Appearance {
    preference: ThemePreference::System,
    theme: STANDARD_THEME_ID.to_string(),
    halves: ThemeHalves::default(),
    custom_themes: Vec::new(),
    interface_font: "geist".to_string(),
    interface_size: INTERFACE_SIZE.default,
    code_font: "jetbrains".to_string(),
    code_size: CODE_SIZE.default,
    separate_fonts: false,
    composer_font: "interface".to_string(),
    composer_size: COMPOSER_SIZE.default,
    terminal_font: "jetbrains".to_string(),
    terminal_size: TERMINAL_SIZE.default,
    word_wrap: true,
    font_smoothing: true,
    lighter_text: true,
});

impl Default for Appearance {
    fn default() -> Self {
        DEFAULT_APPEARANCE.clone()
    }
}

impl Appearance {
    /// Uses `selection` as the theme and its halves.
    pub fn set_selection(&mut self, selection: ThemeSelection) {
        self.theme = selection.theme;
        self.halves = selection.halves;
    }

    pub fn selection(&self) -> ThemeSelection {
        ThemeSelection {
            theme: self.theme.clone(),
            halves: self.halves.clone(),
        }
    }

    /// The camelCase state the web stores and `normalize_appearance` reads.
    pub fn to_json(&self) -> Value {
        json!({
            "preference": self.preference.as_str(),
            "theme": self.theme,
            "halves": self.halves.to_json(),
            "customThemes": self.custom_themes.iter().map(ThemeDefinition::to_json).collect::<Vec<_>>(),
            "interfaceFont": self.interface_font,
            "interfaceSize": self.interface_size,
            "codeFont": self.code_font,
            "codeSize": self.code_size,
            "separateFonts": self.separate_fonts,
            "composerFont": self.composer_font,
            "composerSize": self.composer_size,
            "terminalFont": self.terminal_font,
            "terminalSize": self.terminal_size,
            "wordWrap": self.word_wrap,
            "fontSmoothing": self.font_smoothing,
            "lighterText": self.lighter_text,
        })
    }
}

fn font_choice(value: Option<&Value>, fallback: &str) -> String {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|family| !family.is_empty() && utf16_len(family) <= MAX_FAMILY_LENGTH)
        .unwrap_or(fallback)
        .to_string()
}

fn size(value: Option<&Value>, range: SizeRange) -> u32 {
    value
        .and_then(Value::as_f64)
        .map_or(range.default, |value| clamp_size(value, range))
}

fn flag(value: Option<&Value>, fallback: bool) -> bool {
    value.and_then(Value::as_bool).unwrap_or(fallback)
}

/// Earlier versions saved a built-in palette as `palette`.
fn saved_theme(raw: &serde_json::Map<String, Value>, custom_themes: &[ThemeDefinition]) -> String {
    ["theme", "palette"]
        .into_iter()
        .filter_map(|key| raw.get(key)?.as_str())
        .find_map(|candidate| find_theme(candidate, custom_themes))
        .map_or_else(|| STANDARD_THEME_ID.to_string(), |theme| theme.id.clone())
}

/// Reads what an earlier version saved; unknown or missing fields take their default.
pub fn normalize_appearance(saved: &Value) -> Appearance {
    let empty = serde_json::Map::new();
    let raw = saved.as_object().unwrap_or(&empty);
    let defaults = &*DEFAULT_APPEARANCE;
    let custom_themes = parse_stored_themes(raw.get("customThemes").unwrap_or(&Value::Null));
    Appearance {
        preference: raw
            .get("preference")
            .and_then(Value::as_str)
            .and_then(ThemePreference::parse)
            .unwrap_or(defaults.preference),
        theme: saved_theme(raw, &custom_themes),
        halves: valid_halves(
            &parse_halves(raw.get("halves").unwrap_or(&Value::Null)),
            &custom_themes,
        ),
        interface_font: font_choice(raw.get("interfaceFont"), &defaults.interface_font),
        interface_size: size(raw.get("interfaceSize"), INTERFACE_SIZE),
        code_font: font_choice(raw.get("codeFont"), &defaults.code_font),
        code_size: size(raw.get("codeSize"), CODE_SIZE),
        separate_fonts: flag(raw.get("separateFonts"), defaults.separate_fonts),
        composer_font: font_choice(raw.get("composerFont"), &defaults.composer_font),
        composer_size: size(raw.get("composerSize"), COMPOSER_SIZE),
        terminal_font: font_choice(raw.get("terminalFont"), &defaults.terminal_font),
        terminal_size: size(raw.get("terminalSize"), TERMINAL_SIZE),
        word_wrap: flag(raw.get("wordWrap"), defaults.word_wrap),
        font_smoothing: flag(raw.get("fontSmoothing"), defaults.font_smoothing),
        lighter_text: flag(raw.get("lighterText"), defaults.lighter_text),
        custom_themes,
    }
}

pub fn resolve_appearance(appearance: &Appearance, system_dark: bool) -> ThemeAppearance {
    resolve_theme_appearance(
        &appearance.selection(),
        appearance.preference,
        system_dark,
        &appearance.custom_themes,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::themes::model::{ThemeCollection, derive_theme_colors};

    fn with(update: impl FnOnce(&mut Appearance)) -> Appearance {
        let mut appearance = Appearance::default();
        update(&mut appearance);
        appearance
    }

    fn dusk() -> ThemeDefinition {
        ThemeDefinition::new(
            "dusk",
            "Dusk",
            ThemeAppearance::Dark,
            derive_theme_colors(ThemeAppearance::Dark, "#15131c", "#d08cff"),
        )
    }

    #[test]
    fn keeps_the_theme_an_earlier_version_saved_and_fills_the_rest() {
        assert_eq!(
            normalize_appearance(&json!({ "preference": "dark" })),
            with(|a| a.preference = ThemePreference::Dark)
        );
    }

    #[test]
    fn falls_back_to_the_defaults_without_saved_state() {
        assert_eq!(normalize_appearance(&Value::Null), *DEFAULT_APPEARANCE);
        assert_eq!(normalize_appearance(&json!("dark")), *DEFAULT_APPEARANCE);
        assert_eq!(normalize_appearance(&json!([])), *DEFAULT_APPEARANCE);
    }

    #[test]
    fn drops_values_it_does_not_know() {
        let saved = json!({
            "preference": "sepia",
            "theme": "neon",
            "halves": { "light": "gone" },
            "customThemes": [{ "id": "grove" }],
            "interfaceFont": "   ",
            "codeFont": 7,
            "terminalFont": "x".repeat(201),
            "lighterText": "no"
        });
        assert_eq!(normalize_appearance(&saved), *DEFAULT_APPEARANCE);
    }

    #[test]
    fn keeps_valid_choices() {
        let saved = json!({
            "preference": "light",
            "theme": "iris",
            "halves": { "dark": "ocean" },
            "customThemes": [],
            "interfaceFont": "inter",
            "interfaceSize": 14,
            "codeFont": "cascadia",
            "codeSize": 15,
            "separateFonts": true,
            "composerFont": "jetbrains",
            "composerSize": 15,
            "terminalFont": "system",
            "terminalSize": 11,
            "wordWrap": false,
            "fontSmoothing": false,
            "lighterText": false
        });
        let appearance = normalize_appearance(&saved);
        assert_eq!(appearance.to_json(), saved);
        assert_eq!(appearance.halves.dark.as_deref(), Some("ocean"));
    }

    #[test]
    fn keeps_the_family_name_of_an_installed_font() {
        let appearance = normalize_appearance(&json!({
            "interfaceFont": " Segoe UI ",
            "codeFont": "Fira Code",
            "composerFont": "Iosevka",
            "terminalFont": "Cascadia Code"
        }));
        assert_eq!(appearance.interface_font, "Segoe UI");
        assert_eq!(appearance.code_font, "Fira Code");
        assert_eq!(appearance.composer_font, "Iosevka");
        assert_eq!(appearance.terminal_font, "Cascadia Code");
    }

    #[test]
    fn moves_the_palette_an_earlier_version_saved_to_the_theme() {
        assert_eq!(
            normalize_appearance(&json!({ "palette": "ember" })).theme,
            "ember"
        );
        assert_eq!(
            normalize_appearance(&json!({ "palette": "neon" })).theme,
            "hoshi"
        );
    }

    #[test]
    fn keeps_a_custom_theme_it_can_read() {
        let appearance = normalize_appearance(&json!({
            "theme": "dusk",
            "halves": { "light": "grove", "dark": "dusk" },
            "customThemes": [dusk().to_json(), { "id": "broken" }]
        }));
        assert_eq!(appearance.theme, "dusk");
        assert_eq!(
            appearance.halves,
            ThemeHalves {
                light: Some("grove".into()),
                dark: Some("dusk".into())
            }
        );
        assert_eq!(appearance.custom_themes, vec![dusk()]);
    }

    #[test]
    fn clamps_sizes_into_their_range() {
        let appearance = normalize_appearance(&json!({ "interfaceSize": 40, "codeSize": 2 }));
        assert_eq!(appearance.interface_size, INTERFACE_SIZE.max);
        assert_eq!(appearance.code_size, CODE_SIZE.min);
    }

    #[test]
    fn the_sizes_the_old_sliders_saved_still_apply() {
        let appearance = normalize_appearance(&json!({ "interfaceSize": 17, "codeSize": 12 }));
        assert_eq!(appearance.interface_size, 17);
        assert_eq!(appearance.code_size, 12);
        assert!(!appearance.separate_fonts);
        assert!(appearance.word_wrap);
    }

    #[test]
    fn clamp_size_rounds_to_whole_pixels() {
        assert_eq!(clamp_size(14.6, INTERFACE_SIZE), 15);
        assert_eq!(clamp_size(14.5, INTERFACE_SIZE), 15);
    }

    #[test]
    fn clamp_size_uses_the_default_for_anything_but_a_finite_number() {
        assert_eq!(clamp_size(f64::NAN, CODE_SIZE), CODE_SIZE.default);
        assert_eq!(size(Some(&json!("14")), CODE_SIZE), CODE_SIZE.default);
    }

    #[test]
    fn size_options_list_every_pixel() {
        assert_eq!(size_options(CODE_SIZE), (10..=18).collect::<Vec<_>>());
    }

    #[test]
    fn font_presets_belong_to_their_slot() {
        assert!(is_font_preset(FontSlot::Composer, "system-mono"));
        assert!(is_font_preset(FontSlot::Terminal, "cascadia"));
        assert!(!is_font_preset(FontSlot::Interface, "jetbrains"));
    }

    #[test]
    fn system_follows_the_system_an_explicit_mode_ignores_it() {
        assert_eq!(
            resolve_appearance(&DEFAULT_APPEARANCE, true),
            ThemeAppearance::Dark
        );
        assert_eq!(
            resolve_appearance(&with(|a| a.preference = ThemePreference::Light), true),
            ThemeAppearance::Light
        );
    }

    #[test]
    fn round_trips_through_to_json() {
        let mut custom = dusk();
        custom.managed = true;
        custom.collection = Some(ThemeCollection {
            id: "open-vsx:acme.owls".into(),
            label: "Owls".into(),
        });
        let appearance = with(|a| {
            a.preference = ThemePreference::Dark;
            a.theme = "ocean".into();
            a.halves.dark = Some("dusk".into());
            a.custom_themes = vec![custom];
            a.interface_font = "Segoe UI".into();
            a.interface_size = 15;
            a.separate_fonts = true;
            a.terminal_font = "cascadia".into();
            a.terminal_size = 10;
            a.lighter_text = false;
        });
        let stored = serde_json::to_string(&appearance.to_json()).unwrap();
        let read = normalize_appearance(&serde_json::from_str(&stored).unwrap());
        assert_eq!(read, appearance);
        assert_eq!(
            normalize_appearance(&DEFAULT_APPEARANCE.to_json()),
            *DEFAULT_APPEARANCE
        );
    }
}
