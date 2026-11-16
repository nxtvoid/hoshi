//! Themes, their colors and the light/dark selection (`state/themes.ts`).

use super::appearance::ThemePreference;
use super::color::{
    BLACK, LightnessDirection, Oklch, Rgb, WHITE, canonical_color, color_to_rgb, contrast_ratio,
    is_dark_rgb, mix_rgb, oklch_to_color, oklch_to_rgb, parse_color, readable_foreground,
    readable_on, rgb_to_color, rgb_to_oklch, softened_text, solve_lightness,
};
use serde_json::{Map, Value, json};
use std::fmt;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemeAppearance {
    Light,
    Dark,
}

impl ThemeAppearance {
    pub const ALL: [ThemeAppearance; 2] = [ThemeAppearance::Light, ThemeAppearance::Dark];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }

    pub fn other(self) -> Self {
        match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
        }
    }
}

pub const THEME_COLOR_ROLES: [&str; 35] = [
    "background",
    "foreground",
    "card",
    "cardForeground",
    "popover",
    "popoverForeground",
    "primary",
    "primaryForeground",
    "secondary",
    "secondaryForeground",
    "muted",
    "mutedForeground",
    "accent",
    "accentForeground",
    "destructive",
    "border",
    "input",
    "ring",
    "sidebar",
    "sidebarForeground",
    "sidebarPrimary",
    "sidebarPrimaryForeground",
    "sidebarAccent",
    "sidebarAccentForeground",
    "sidebarBorder",
    "sidebarRing",
    "header",
    "headerGlow",
    "link",
    "send",
    "statusError",
    "statusWaiting",
    "terminalSelection",
    "scrollbar",
    "scrollbarHover",
];

const ROLE_COUNT: usize = THEME_COLOR_ROLES.len();

pub fn is_theme_color_role(role: &str) -> bool {
    role_index(role).is_some()
}

fn role_index(role: &str) -> Option<usize> {
    THEME_COLOR_ROLES.iter().position(|known| *known == role)
}

/// The token a role sets, `sidebarBorder` → `sidebar-border`.
pub fn role_variable(role: &str) -> String {
    let mut variable = String::with_capacity(role.len() + 4);
    for ch in role.chars() {
        if ch.is_ascii_uppercase() {
            variable.push('-');
            variable.push(ch.to_ascii_lowercase());
        } else {
            variable.push(ch);
        }
    }
    variable
}

/// One CSS color string per role in `THEME_COLOR_ROLES`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeColors {
    values: [String; ROLE_COUNT],
}

impl ThemeColors {
    fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        let mut values: [Option<String>; ROLE_COUNT] = std::array::from_fn(|_| None);
        for (role, value) in pairs {
            let index = role_index(role).unwrap_or_else(|| panic!("unknown role {role}"));
            values[index] = Some((*value).to_string());
        }
        Self {
            values: values.map(|value| value.expect("every role has a color")),
        }
    }

    /// The color of `role`; empty for a name that isn't a role.
    pub fn get(&self, role: &str) -> &str {
        debug_assert!(is_theme_color_role(role), "unknown role {role}");
        role_index(role).map_or("", |index| &self.values[index])
    }

    pub fn set(&mut self, role: &str, value: impl Into<String>) {
        debug_assert!(is_theme_color_role(role), "unknown role {role}");
        if let Some(index) = role_index(role) {
            self.values[index] = value.into();
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &str)> {
        THEME_COLOR_ROLES
            .iter()
            .zip(self.values.iter())
            .map(|(role, value)| (*role, value.as_str()))
    }

    /// `{ role: color }`, every role.
    pub fn to_json(&self) -> Value {
        Value::Object(
            self.iter()
                .map(|(role, value)| (role.to_string(), Value::String(value.to_string())))
                .collect(),
        )
    }
}

/// The colors a theme has for its other appearance.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThemeVariants {
    pub light: Option<ThemeColors>,
    pub dark: Option<ThemeColors>,
}

impl ThemeVariants {
    pub fn get(&self, appearance: ThemeAppearance) -> Option<&ThemeColors> {
        match appearance {
            ThemeAppearance::Light => self.light.as_ref(),
            ThemeAppearance::Dark => self.dark.as_ref(),
        }
    }

    pub fn set(&mut self, appearance: ThemeAppearance, colors: Option<ThemeColors>) {
        match appearance {
            ThemeAppearance::Light => self.light = colors,
            ThemeAppearance::Dark => self.dark = colors,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.light.is_none() && self.dark.is_none()
    }

    fn to_json(&self) -> Value {
        let mut map = Map::new();
        for appearance in ThemeAppearance::ALL {
            if let Some(colors) = self.get(appearance) {
                map.insert(appearance.as_str().to_string(), colors.to_json());
            }
        }
        Value::Object(map)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeCollection {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeDefinition {
    pub id: String,
    pub label: String,
    pub appearance: ThemeAppearance,
    pub colors: ThemeColors,
    pub variants: ThemeVariants,
    pub collection: Option<ThemeCollection>,
    /// Made from a background and an accent, so the editor reopens in its simple mode.
    pub managed: bool,
}

impl ThemeDefinition {
    /// A theme with one appearance and nothing else.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        appearance: ThemeAppearance,
        colors: ThemeColors,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            appearance,
            colors,
            variants: ThemeVariants::default(),
            collection: None,
            managed: false,
        }
    }

    pub fn variant(&self, appearance: ThemeAppearance) -> Option<&ThemeColors> {
        self.variants.get(appearance)
    }

    /// The shape themes are stored in: `{ id, label, appearance, colors, variants?, collection?, managed? }`.
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        map.insert("id".into(), json!(self.id));
        map.insert("label".into(), json!(self.label));
        map.insert("appearance".into(), json!(self.appearance.as_str()));
        map.insert("colors".into(), self.colors.to_json());
        if !self.variants.is_empty() {
            map.insert("variants".into(), self.variants.to_json());
        }
        if let Some(collection) = &self.collection {
            map.insert(
                "collection".into(),
                json!({ "id": collection.id, "label": collection.label }),
            );
        }
        if self.managed {
            map.insert("managed".into(), Value::Bool(true));
        }
        Value::Object(map)
    }
}

pub const STANDARD_THEME_ID: &str = "hoshi";
pub const BUILT_IN_PALETTE_IDS: [&str; 4] = ["grove", "ocean", "ember", "iris"];

/// Custom themes can't take these, or they would capture a stored choice.
pub const RESERVED_THEME_IDS: [&str; 8] = [
    "system",
    "light",
    "dark",
    STANDARD_THEME_ID,
    "grove",
    "ocean",
    "ember",
    "iris",
];

pub const THEME_NAME_MAX: usize = 48;
pub const THEME_FILE_VERSION: u64 = 1;

pub fn is_reserved_theme_id(id: &str) -> bool {
    RESERVED_THEME_IDS.contains(&id)
}

/// `crates/hoshi-ui/assets/globals.css`, `:root`.
static STANDARD_LIGHT: LazyLock<ThemeColors> = LazyLock::new(|| {
    ThemeColors::from_pairs(&[
        ("background", "oklch(1 0 0)"),
        ("foreground", "oklch(0.145 0 0)"),
        ("card", "oklch(1 0 0)"),
        ("cardForeground", "oklch(0.145 0 0)"),
        ("popover", "oklch(1 0 0)"),
        ("popoverForeground", "oklch(0.145 0 0)"),
        ("primary", "oklch(0.205 0 0)"),
        ("primaryForeground", "oklch(0.985 0 0)"),
        ("secondary", "oklch(0.97 0 0)"),
        ("secondaryForeground", "oklch(0.205 0 0)"),
        ("muted", "oklch(0.97 0 0)"),
        ("mutedForeground", "oklch(0.556 0 0)"),
        ("accent", "oklch(0.97 0 0)"),
        ("accentForeground", "oklch(0.205 0 0)"),
        ("destructive", "oklch(0.577 0.245 27.325)"),
        ("border", "oklch(0.922 0 0)"),
        ("input", "oklch(0.922 0 0)"),
        ("ring", "oklch(0.708 0 0)"),
        ("sidebar", "oklch(0.985 0 0)"),
        ("sidebarForeground", "oklch(0.145 0 0)"),
        ("sidebarPrimary", "oklch(0.205 0 0)"),
        ("sidebarPrimaryForeground", "oklch(0.985 0 0)"),
        ("sidebarAccent", "oklch(0.97 0 0)"),
        ("sidebarAccentForeground", "oklch(0.205 0 0)"),
        ("sidebarBorder", "oklch(0.922 0 0)"),
        ("sidebarRing", "oklch(0.708 0 0)"),
        ("header", "oklch(0.975 0 0)"),
        ("headerGlow", "oklch(0.85 0 0 / 0.35)"),
        ("link", "oklch(0.55 0.17 252)"),
        ("send", "oklch(0.56 0.16 250)"),
        ("statusError", "oklch(0.6 0.21 25)"),
        ("statusWaiting", "oklch(0.72 0.16 70)"),
        ("terminalSelection", "oklch(0.708 0 0 / 0.35)"),
        ("scrollbar", "oklch(0.145 0 0 / 0.18)"),
        ("scrollbarHover", "oklch(0.145 0 0 / 0.34)"),
    ])
});

/// `crates/hoshi-ui/assets/globals.css`, `.dark`.
static STANDARD_DARK: LazyLock<ThemeColors> = LazyLock::new(|| {
    ThemeColors::from_pairs(&[
        ("background", "oklch(0.145 0 0)"),
        ("foreground", "oklch(0.985 0 0)"),
        ("card", "oklch(0.205 0 0)"),
        ("cardForeground", "oklch(0.985 0 0)"),
        ("popover", "oklch(0.205 0 0)"),
        ("popoverForeground", "oklch(0.985 0 0)"),
        ("primary", "oklch(0.922 0 0)"),
        ("primaryForeground", "oklch(0.205 0 0)"),
        ("secondary", "oklch(0.269 0 0)"),
        ("secondaryForeground", "oklch(0.985 0 0)"),
        ("muted", "oklch(0.269 0 0)"),
        ("mutedForeground", "oklch(0.708 0 0)"),
        ("accent", "oklch(0.269 0 0)"),
        ("accentForeground", "oklch(0.985 0 0)"),
        ("destructive", "oklch(0.704 0.191 22.216)"),
        ("border", "oklch(1 0 0 / 10%)"),
        ("input", "oklch(1 0 0 / 15%)"),
        ("ring", "oklch(0.556 0 0)"),
        ("sidebar", "oklch(0.205 0 0)"),
        ("sidebarForeground", "oklch(0.985 0 0)"),
        ("sidebarPrimary", "oklch(0.488 0.243 264.376)"),
        ("sidebarPrimaryForeground", "oklch(0.985 0 0)"),
        ("sidebarAccent", "oklch(0.269 0 0)"),
        ("sidebarAccentForeground", "oklch(0.985 0 0)"),
        ("sidebarBorder", "oklch(1 0 0 / 10%)"),
        ("sidebarRing", "oklch(0.556 0 0)"),
        ("header", "oklch(0.19 0 0)"),
        ("headerGlow", "oklch(0.85 0 0 / 0.08)"),
        ("link", "oklch(0.74 0.13 245)"),
        ("send", "oklch(0.74 0.13 245)"),
        ("statusError", "oklch(0.7 0.18 22)"),
        ("statusWaiting", "oklch(0.83 0.15 80)"),
        ("terminalSelection", "oklch(0.556 0 0 / 0.45)"),
        ("scrollbar", "oklch(0.985 0 0 / 0.16)"),
        ("scrollbarHover", "oklch(0.985 0 0 / 0.3)"),
    ])
});

pub fn standard_colors(appearance: ThemeAppearance) -> &'static ThemeColors {
    match appearance {
        ThemeAppearance::Light => &STANDARD_LIGHT,
        ThemeAppearance::Dark => &STANDARD_DARK,
    }
}

/// The hue and chroma a built-in palette tints with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaletteTint {
    pub hue: f64,
    pub chroma: f64,
}

/// `[data-palette]` in `globals.css`.
pub fn palette_tint(palette: &str) -> Option<PaletteTint> {
    let (hue, chroma) = match palette {
        "grove" => (158.0, 0.11),
        "ocean" => (245.0, 0.12),
        "ember" => (48.0, 0.13),
        "iris" => (294.0, 0.15),
        _ => return None,
    };
    Some(PaletteTint { hue, chroma })
}

pub fn is_built_in_palette(id: &str) -> bool {
    BUILT_IN_PALETTE_IDS.contains(&id)
}

/// The colors `[data-palette]` and `.dark[data-palette]` in `globals.css` paint.
pub fn palette_colors(palette: &str, appearance: ThemeAppearance) -> Option<ThemeColors> {
    let PaletteTint { hue: h, chroma } = palette_tint(palette)?;
    let at = |l: f64, c: f64| format!("oklch({l} {c} {h})");
    let at_alpha = |l: f64, c: f64, alpha: f64| format!("oklch({l} {c} {h} / {alpha})");
    let mut colors = standard_colors(appearance).clone();
    let overrides: Vec<(&str, String)> = match appearance {
        ThemeAppearance::Light => vec![
            ("background", at(0.988, 0.004)),
            ("card", at(0.995, 0.003)),
            ("popover", at(0.995, 0.003)),
            ("primary", at(0.52, chroma)),
            ("primaryForeground", at(0.985, 0.005)),
            ("secondary", at(0.96, 0.01)),
            ("secondaryForeground", at(0.24, 0.03)),
            ("muted", at(0.96, 0.01)),
            ("mutedForeground", at(0.53, 0.02)),
            ("accent", at(0.95, 0.018)),
            ("accentForeground", at(0.24, 0.03)),
            ("border", at(0.915, 0.01)),
            ("input", at(0.915, 0.01)),
            ("ring", at(0.62, 0.1)),
            ("sidebar", at(0.972, 0.01)),
            ("sidebarPrimary", at(0.52, chroma)),
            ("sidebarPrimaryForeground", at(0.985, 0.005)),
            ("sidebarAccent", at(0.94, 0.02)),
            ("sidebarAccentForeground", at(0.24, 0.03)),
            ("sidebarBorder", at(0.915, 0.012)),
            ("sidebarRing", at(0.62, 0.1)),
            ("header", at(0.962, 0.012)),
            ("headerGlow", at_alpha(0.85, 0.06, 0.35)),
            ("terminalSelection", at_alpha(0.7, 0.08, 0.35)),
        ],
        ThemeAppearance::Dark => vec![
            ("background", at(0.16, 0.01)),
            ("card", at(0.215, 0.014)),
            ("popover", at(0.215, 0.014)),
            ("primary", at(0.78, chroma)),
            ("primaryForeground", at(0.2, 0.03)),
            ("secondary", at(0.27, 0.016)),
            ("secondaryForeground", at(0.985, 0.004)),
            ("muted", at(0.27, 0.016)),
            ("mutedForeground", at(0.72, 0.015)),
            ("accent", at(0.28, 0.022)),
            ("accentForeground", at(0.985, 0.004)),
            ("ring", at(0.58, 0.09)),
            ("sidebar", at(0.2, 0.014)),
            ("sidebarPrimary", at(0.78, chroma)),
            ("sidebarPrimaryForeground", at(0.2, 0.03)),
            ("sidebarAccent", at(0.28, 0.022)),
            ("sidebarAccentForeground", at(0.985, 0.004)),
            ("sidebarRing", at(0.58, 0.09)),
            ("header", at(0.19, 0.012)),
            ("headerGlow", at_alpha(0.85, 0.06, 0.08)),
            ("terminalSelection", at_alpha(0.6, 0.08, 0.45)),
        ],
    };
    for (role, value) in overrides {
        colors.set(role, value);
    }
    Some(colors)
}

fn built_in_theme(id: &str, label: &str) -> ThemeDefinition {
    let colors = |appearance| {
        if id == STANDARD_THEME_ID {
            standard_colors(appearance).clone()
        } else {
            palette_colors(id, appearance).expect("a built-in palette")
        }
    };
    ThemeDefinition {
        variants: ThemeVariants {
            light: None,
            dark: Some(colors(ThemeAppearance::Dark)),
        },
        ..ThemeDefinition::new(
            id,
            label,
            ThemeAppearance::Light,
            colors(ThemeAppearance::Light),
        )
    }
}

static BUILT_IN_THEMES: LazyLock<Vec<ThemeDefinition>> = LazyLock::new(|| {
    vec![
        built_in_theme(STANDARD_THEME_ID, "Hoshi"),
        built_in_theme("grove", "Grove"),
        built_in_theme("ocean", "Ocean"),
        built_in_theme("ember", "Ember"),
        built_in_theme("iris", "Iris"),
    ]
});

/// Shipped themes; their labels are proper names, the same in every locale.
pub fn built_in_themes() -> &'static [ThemeDefinition] {
    &BUILT_IN_THEMES
}

pub fn find_theme<'a>(
    id: &str,
    custom_themes: &'a [ThemeDefinition],
) -> Option<&'a ThemeDefinition> {
    built_in_themes()
        .iter()
        .chain(custom_themes)
        .find(|theme| theme.id == id)
}

pub fn theme_colors_for(
    theme: &ThemeDefinition,
    appearance: ThemeAppearance,
) -> Option<&ThemeColors> {
    if appearance == theme.appearance {
        return Some(&theme.colors);
    }
    theme.variant(appearance)
}

pub fn theme_modes(theme: &ThemeDefinition) -> Vec<ThemeAppearance> {
    ThemeAppearance::ALL
        .into_iter()
        .filter(|appearance| theme_colors_for(theme, *appearance).is_some())
        .collect()
}

/// The one appearance a theme has, or `None` when it has both.
pub fn single_appearance_of(theme: &ThemeDefinition) -> Option<ThemeAppearance> {
    match theme_modes(theme).as_slice() {
        [single] => Some(*single),
        _ => None,
    }
}

pub fn other_appearance(appearance: ThemeAppearance) -> ThemeAppearance {
    appearance.other()
}

pub(crate) fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The longest start of `text` within `max` UTF-16 units, as `String.prototype.slice` cuts it.
pub(crate) fn utf16_prefix(text: &str, max: usize) -> &str {
    let mut units = 0;
    for (index, ch) in text.char_indices() {
        units += ch.len_utf16();
        if units > max {
            return &text[..index];
        }
    }
    text
}

fn to_radix_36(mut value: u32) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut digits = Vec::new();
    loop {
        digits.push(DIGITS[(value % 36) as usize]);
        value /= 36;
        if value == 0 {
            break;
        }
    }
    digits.reverse();
    String::from_utf8(digits).unwrap_or_default()
}

pub fn theme_id_from_name(name: &str) -> String {
    let label = name.trim().to_lowercase();
    let mut slug = String::new();
    for ch in label.chars() {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            slug.push(ch);
        } else if !slug.ends_with('-') || slug.is_empty() {
            slug.push('-');
        }
    }
    let normalized: String = slug
        .trim_matches('-')
        .chars()
        .take(THEME_NAME_MAX)
        .collect();
    if !normalized.is_empty() {
        return normalized;
    }
    if label.is_empty() {
        return "custom-theme".to_string();
    }
    let mut hash: u32 = 2_166_136_261;
    for unit in label.encode_utf16() {
        hash ^= u32::from(unit);
        hash = hash.wrapping_mul(16_777_619);
    }
    format!("theme-{}", to_radix_36(hash))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemeErrorCode {
    NotAnObject,
    UnsupportedVersion,
    MissingName,
    MissingAppearance,
    MissingColors,
    InvalidId,
    ReservedId,
    UnknownRole,
    InvalidColor,
    NoColors,
    InvalidCollection,
    InvalidVariants,
    VariantRepeatsAppearance,
    AlreadyInstalled,
    NotInstalled,
    EmptyCollection,
    InvalidCollectionThemes,
    PaletteTaken,
    PalettesTaken,
    CollectionChanged,
    VscodeNoBackground,
    InvalidJson,
    NoCompatibleThemes,
}

impl ThemeErrorCode {
    /// The web's code, `not_an_object`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotAnObject => "not_an_object",
            Self::UnsupportedVersion => "unsupported_version",
            Self::MissingName => "missing_name",
            Self::MissingAppearance => "missing_appearance",
            Self::MissingColors => "missing_colors",
            Self::InvalidId => "invalid_id",
            Self::ReservedId => "reserved_id",
            Self::UnknownRole => "unknown_role",
            Self::InvalidColor => "invalid_color",
            Self::NoColors => "no_colors",
            Self::InvalidCollection => "invalid_collection",
            Self::InvalidVariants => "invalid_variants",
            Self::VariantRepeatsAppearance => "variant_repeats_appearance",
            Self::AlreadyInstalled => "already_installed",
            Self::NotInstalled => "not_installed",
            Self::EmptyCollection => "empty_collection",
            Self::InvalidCollectionThemes => "invalid_collection_themes",
            Self::PaletteTaken => "palette_taken",
            Self::PalettesTaken => "palettes_taken",
            Self::CollectionChanged => "collection_changed",
            Self::VscodeNoBackground => "vscode_no_background",
            Self::InvalidJson => "invalid_json",
            Self::NoCompatibleThemes => "no_compatible_themes",
        }
    }
}

/// A theme that can't be read or saved; the UI words `code`, `subject` names what it's about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeError {
    pub code: ThemeErrorCode,
    pub subject: Option<String>,
    pub appearance: Option<ThemeAppearance>,
}

impl ThemeError {
    pub fn new(code: ThemeErrorCode) -> Self {
        Self {
            code,
            subject: None,
            appearance: None,
        }
    }

    pub fn about(code: ThemeErrorCode, subject: impl Into<String>) -> Self {
        Self {
            subject: Some(subject.into()),
            ..Self::new(code)
        }
    }
}

impl fmt::Display for ThemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.subject {
            Some(subject) if !subject.is_empty() => write!(f, "{}: {subject}", self.code.as_str()),
            _ => f.write_str(self.code.as_str()),
        }
    }
}

impl std::error::Error for ThemeError {}

fn is_theme_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=48).contains(&bytes.len())
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}

fn theme_label(value: Option<&Value>) -> Option<&str> {
    let label = value?.as_str()?.trim();
    (!label.is_empty() && utf16_len(label) <= THEME_NAME_MAX).then_some(label)
}

fn parse_collection(value: Option<&Value>) -> Option<ThemeCollection> {
    let record = value?.as_object()?;
    let id = record.get("id")?.as_str()?;
    let bytes = id.as_bytes();
    let valid_id = (1..=128).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b':' | b'-'));
    if !valid_id {
        return None;
    }
    let label = theme_label(record.get("label"))?;
    Some(ThemeCollection {
        id: id.to_string(),
        label: label.to_string(),
    })
}

fn appearance_of(value: Option<&Value>) -> Option<ThemeAppearance> {
    value?.as_str().and_then(ThemeAppearance::parse)
}

fn lenient_colors(value: &Map<String, Value>, appearance: ThemeAppearance) -> ThemeColors {
    let mut colors = standard_colors(appearance).clone();
    for (role, color) in value {
        if !is_theme_color_role(role) {
            continue;
        }
        if let Some(normalized) = color.as_str().and_then(canonical_color) {
            colors.set(role, normalized);
        }
    }
    colors
}

/// Reads one theme from storage, skipping unknown roles; `None` when it can't be used.
pub fn parse_stored_theme(value: &Value) -> Option<ThemeDefinition> {
    let record = value.as_object()?;
    let id = record.get("id")?.as_str()?;
    if !is_theme_id(id) || is_reserved_theme_id(id) {
        return None;
    }
    let label = theme_label(record.get("label"))?;
    let appearance = appearance_of(record.get("appearance"))?;
    let colors = lenient_colors(record.get("colors")?.as_object()?, appearance);
    let mut variants = ThemeVariants::default();
    if let Some(stored) = record.get("variants") {
        for (key, variant) in stored.as_object()? {
            let variant_appearance = ThemeAppearance::parse(key)?;
            let variant = variant.as_object()?;
            if variant_appearance == appearance {
                continue;
            }
            variants.set(
                variant_appearance,
                Some(lenient_colors(variant, variant_appearance)),
            );
        }
    }
    Some(ThemeDefinition {
        id: id.to_string(),
        label: label.to_string(),
        appearance,
        colors,
        variants,
        collection: parse_collection(record.get("collection")),
        managed: record.get("managed") == Some(&Value::Bool(true)),
    })
}

pub fn parse_stored_themes(value: &Value) -> Vec<ThemeDefinition> {
    let Some(stored) = value.as_array() else {
        return Vec::new();
    };
    let mut themes: Vec<ThemeDefinition> = Vec::new();
    for entry in stored {
        if let Some(theme) = parse_stored_theme(entry)
            && !themes.iter().any(|existing| existing.id == theme.id)
        {
            themes.push(theme);
        }
    }
    themes
}

fn strict_colors(value: &Value, appearance: ThemeAppearance) -> Result<ThemeColors, ThemeError> {
    let record = value
        .as_object()
        .ok_or(ThemeError::new(ThemeErrorCode::MissingColors))?;
    let mut colors = standard_colors(appearance).clone();
    for (role, color) in record {
        if !is_theme_color_role(role) {
            return Err(ThemeError::about(ThemeErrorCode::UnknownRole, role));
        }
        let normalized = color
            .as_str()
            .and_then(canonical_color)
            .ok_or_else(|| ThemeError::about(ThemeErrorCode::InvalidColor, role))?;
        colors.set(role, normalized);
    }
    if record.is_empty() {
        return Err(ThemeError::new(ThemeErrorCode::NoColors));
    }
    Ok(colors)
}

/// Reads a hoshi theme file, `{ version: 1, name, appearance, colors, variants? }`.
pub fn parse_theme_file(value: &Value) -> Result<ThemeDefinition, ThemeError> {
    let record = value
        .as_object()
        .ok_or(ThemeError::new(ThemeErrorCode::NotAnObject))?;
    if record.get("version").and_then(Value::as_f64) != Some(THEME_FILE_VERSION as f64) {
        return Err(ThemeError::new(ThemeErrorCode::UnsupportedVersion));
    }
    let name =
        theme_label(record.get("name")).ok_or(ThemeError::new(ThemeErrorCode::MissingName))?;
    let appearance = appearance_of(record.get("appearance"))
        .ok_or(ThemeError::new(ThemeErrorCode::MissingAppearance))?;
    let colors_value = record
        .get("colors")
        .filter(|colors| colors.is_object())
        .ok_or(ThemeError::new(ThemeErrorCode::MissingColors))?;
    let id = match record.get("id") {
        None => theme_id_from_name(name),
        Some(id) => id
            .as_str()
            .filter(|id| is_theme_id(id))
            .ok_or(ThemeError::new(ThemeErrorCode::InvalidId))?
            .to_string(),
    };
    if !is_theme_id(&id) {
        return Err(ThemeError::new(ThemeErrorCode::InvalidId));
    }
    if is_reserved_theme_id(&id) {
        return Err(ThemeError::about(ThemeErrorCode::ReservedId, id));
    }
    let collection = parse_collection(record.get("collection"));
    if record.contains_key("collection") && collection.is_none() {
        return Err(ThemeError::new(ThemeErrorCode::InvalidCollection));
    }
    let mut variants = ThemeVariants::default();
    if let Some(stored) = record.get("variants") {
        let stored = stored
            .as_object()
            .ok_or(ThemeError::new(ThemeErrorCode::InvalidVariants))?;
        for (key, colors) in stored {
            let variant = ThemeAppearance::parse(key)
                .ok_or(ThemeError::new(ThemeErrorCode::InvalidVariants))?;
            if variant == appearance {
                return Err(ThemeError::about(
                    ThemeErrorCode::VariantRepeatsAppearance,
                    appearance.as_str(),
                ));
            }
            variants.set(variant, Some(strict_colors(colors, variant)?));
        }
    }
    Ok(ThemeDefinition {
        id,
        label: name.to_string(),
        appearance,
        colors: strict_colors(colors_value, appearance)?,
        variants,
        collection,
        managed: record.get("managed") == Some(&Value::Bool(true)),
    })
}

/// The JSON a theme file holds, keys in the order the web writes them.
pub fn theme_file_json(theme: &ThemeDefinition) -> Value {
    let mut file = theme.to_json();
    if let Some(record) = file.as_object_mut() {
        record.remove("label");
        record.insert("version".into(), json!(THEME_FILE_VERSION));
        record.insert("name".into(), json!(theme.label));
    }
    file
}

fn push_indent(out: &mut String, depth: usize) {
    out.push('\n');
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn push_string(out: &mut String, value: &str) {
    out.push_str(&Value::String(value.to_string()).to_string());
}

fn push_colors(out: &mut String, colors: &ThemeColors, depth: usize) {
    out.push('{');
    for (index, (role, value)) in colors.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_indent(out, depth + 1);
        push_string(out, role);
        out.push_str(": ");
        push_string(out, value);
    }
    push_indent(out, depth);
    out.push('}');
}

/// The theme as a file, written like `JSON.stringify(file, null, 2)` with a final newline.
pub fn serialize_theme_file(theme: &ThemeDefinition) -> String {
    let mut out = String::from("{");
    let field = |out: &mut String, key: &str| {
        if out.len() > 1 {
            out.push(',');
        }
        push_indent(out, 1);
        push_string(out, key);
        out.push_str(": ");
    };
    field(&mut out, "version");
    out.push_str(&THEME_FILE_VERSION.to_string());
    field(&mut out, "id");
    push_string(&mut out, &theme.id);
    field(&mut out, "name");
    push_string(&mut out, &theme.label);
    field(&mut out, "appearance");
    push_string(&mut out, theme.appearance.as_str());
    field(&mut out, "colors");
    push_colors(&mut out, &theme.colors, 1);
    if !theme.variants.is_empty() {
        field(&mut out, "variants");
        out.push('{');
        let mut first = true;
        for appearance in ThemeAppearance::ALL {
            let Some(colors) = theme.variant(appearance) else {
                continue;
            };
            if !first {
                out.push(',');
            }
            first = false;
            push_indent(&mut out, 2);
            push_string(&mut out, appearance.as_str());
            out.push_str(": ");
            push_colors(&mut out, colors, 2);
        }
        push_indent(&mut out, 1);
        out.push('}');
    }
    if let Some(collection) = &theme.collection {
        field(&mut out, "collection");
        out.push('{');
        push_indent(&mut out, 2);
        out.push_str("\"id\": ");
        push_string(&mut out, &collection.id);
        out.push(',');
        push_indent(&mut out, 2);
        out.push_str("\"label\": ");
        push_string(&mut out, &collection.label);
        push_indent(&mut out, 1);
        out.push('}');
    }
    if theme.managed {
        field(&mut out, "managed");
        out.push_str("true");
    }
    out.push_str("\n}\n");
    out
}

pub fn install_theme(
    themes: &[ThemeDefinition],
    theme: ThemeDefinition,
) -> Result<Vec<ThemeDefinition>, ThemeError> {
    if is_reserved_theme_id(&theme.id) {
        return Err(ThemeError::about(ThemeErrorCode::ReservedId, theme.id));
    }
    if themes.iter().any(|existing| existing.id == theme.id) {
        return Err(ThemeError::about(
            ThemeErrorCode::AlreadyInstalled,
            theme.label,
        ));
    }
    let mut next = themes.to_vec();
    next.push(theme);
    Ok(next)
}

pub fn update_theme(
    themes: &[ThemeDefinition],
    theme: ThemeDefinition,
) -> Result<Vec<ThemeDefinition>, ThemeError> {
    let index = themes
        .iter()
        .position(|existing| existing.id == theme.id)
        .ok_or_else(|| ThemeError::about(ThemeErrorCode::NotInstalled, theme.label.clone()))?;
    let mut next = themes.to_vec();
    next[index] = theme;
    Ok(next)
}

pub fn remove_themes(themes: &[ThemeDefinition], ids: &[impl AsRef<str>]) -> Vec<ThemeDefinition> {
    themes
        .iter()
        .filter(|theme| !ids.iter().any(|id| id.as_ref() == theme.id))
        .cloned()
        .collect()
}

fn in_collection(theme: &ThemeDefinition, collection_id: &str) -> bool {
    theme
        .collection
        .as_ref()
        .is_some_and(|collection| collection.id == collection_id)
}

/// Swaps an imported collection for its new themes, where the old ones stood.
pub fn replace_collection(
    themes: &[ThemeDefinition],
    collection_id: &str,
    replacement: Vec<ThemeDefinition>,
) -> Result<Vec<ThemeDefinition>, ThemeError> {
    if replacement.is_empty() {
        return Err(ThemeError::new(ThemeErrorCode::EmptyCollection));
    }
    let foreign = replacement
        .iter()
        .any(|theme| !in_collection(theme, collection_id));
    let repeated = replacement.iter().enumerate().any(|(index, theme)| {
        replacement[..index]
            .iter()
            .any(|other| other.id == theme.id)
    });
    if foreign || repeated {
        return Err(ThemeError::new(ThemeErrorCode::InvalidCollectionThemes));
    }
    let others: Vec<&ThemeDefinition> = themes
        .iter()
        .filter(|theme| !in_collection(theme, collection_id))
        .collect();
    let taken = replacement.iter().find(|theme| {
        is_reserved_theme_id(&theme.id) || others.iter().any(|other| other.id == theme.id)
    });
    if let Some(taken) = taken {
        return Err(ThemeError::about(
            ThemeErrorCode::AlreadyInstalled,
            taken.label.clone(),
        ));
    }
    let Some(at) = themes
        .iter()
        .position(|theme| in_collection(theme, collection_id))
    else {
        return Ok(others.into_iter().cloned().chain(replacement).collect());
    };
    let outside = |slice: &[ThemeDefinition]| -> Vec<ThemeDefinition> {
        slice
            .iter()
            .filter(|theme| !in_collection(theme, collection_id))
            .cloned()
            .collect()
    };
    let mut next = outside(&themes[..at]);
    next.extend(replacement);
    next.extend(outside(&themes[at..]));
    Ok(next)
}

/// Light and dark each picked from a different theme. A missing half follows the base theme.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThemeHalves {
    pub light: Option<String>,
    pub dark: Option<String>,
}

impl ThemeHalves {
    pub fn get(&self, appearance: ThemeAppearance) -> Option<&str> {
        match appearance {
            ThemeAppearance::Light => self.light.as_deref(),
            ThemeAppearance::Dark => self.dark.as_deref(),
        }
    }

    pub fn set(&mut self, appearance: ThemeAppearance, id: Option<String>) {
        match appearance {
            ThemeAppearance::Light => self.light = id,
            ThemeAppearance::Dark => self.dark = id,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.light.is_none() && self.dark.is_none()
    }

    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        for appearance in ThemeAppearance::ALL {
            if let Some(id) = self.get(appearance) {
                map.insert(appearance.as_str().to_string(), json!(id));
            }
        }
        Value::Object(map)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThemeSelection {
    pub theme: String,
    pub halves: ThemeHalves,
}

pub fn parse_halves(value: &Value) -> ThemeHalves {
    let mut halves = ThemeHalves::default();
    if let Some(record) = value.as_object() {
        for appearance in ThemeAppearance::ALL {
            if let Some(id) = record.get(appearance.as_str()).and_then(Value::as_str) {
                halves.set(appearance, Some(id.to_string()));
            }
        }
    }
    halves
}

/// The halves that still name a theme with colors for their appearance.
pub fn valid_halves(halves: &ThemeHalves, custom_themes: &[ThemeDefinition]) -> ThemeHalves {
    let mut valid = ThemeHalves::default();
    for appearance in ThemeAppearance::ALL {
        let Some(id) = halves.get(appearance) else {
            continue;
        };
        if let Some(theme) = find_theme(id, custom_themes)
            && theme_colors_for(theme, appearance).is_some()
        {
            valid.set(appearance, Some(theme.id.clone()));
        }
    }
    valid
}

/// Which card owns each half: `None` is the standard theme.
pub fn half_owners(selection: &ThemeSelection, custom_themes: &[ThemeDefinition]) -> ThemeHalves {
    let base_id = find_theme(&selection.theme, custom_themes)
        .filter(|base| base.id != STANDARD_THEME_ID)
        .map(|base| base.id.clone());
    let halves = valid_halves(&selection.halves, custom_themes);
    let owner = |id: Option<&str>| match id {
        None => base_id.clone(),
        Some(STANDARD_THEME_ID) => None,
        Some(id) => Some(id.to_string()),
    };
    ThemeHalves {
        light: owner(halves.get(ThemeAppearance::Light)),
        dark: owner(halves.get(ThemeAppearance::Dark)),
    }
}

/// A whole theme replaces the mix.
pub fn choose_theme(id: &str) -> ThemeSelection {
    ThemeSelection {
        theme: id.to_string(),
        halves: ThemeHalves::default(),
    }
}

/// Gives one half to a card; `None` gives it back to the standard theme.
pub fn choose_half(
    selection: &ThemeSelection,
    appearance: ThemeAppearance,
    card_id: Option<&str>,
) -> ThemeSelection {
    let other = appearance.other();
    if card_id.is_none() && selection.theme != STANDARD_THEME_ID {
        let other_owner = selection.halves.get(other).unwrap_or(&selection.theme);
        let mut halves = ThemeHalves::default();
        halves.set(other, Some(other_owner.to_string()));
        return ThemeSelection {
            theme: STANDARD_THEME_ID.to_string(),
            halves,
        };
    }
    let mut halves = selection.halves.clone();
    halves.set(appearance, card_id.map(str::to_string));
    ThemeSelection {
        theme: selection.theme.clone(),
        halves,
    }
}

/// A card click: a theme with one appearance takes that half, any other the whole.
pub fn choose_card(selection: &ThemeSelection, theme: &ThemeDefinition) -> ThemeSelection {
    match single_appearance_of(theme) {
        None => choose_theme(&theme.id),
        Some(single) => choose_half(selection, single, Some(&theme.id)),
    }
}

/// The selection once `ids` are gone: their halves fall back to the standard theme.
pub fn selection_without(selection: &ThemeSelection, ids: &[impl AsRef<str>]) -> ThemeSelection {
    let removed = |id: &str| ids.iter().any(|removed| removed.as_ref() == id);
    let removes_base = removed(&selection.theme);
    let mut halves = if removes_base {
        ThemeHalves::default()
    } else {
        selection.halves.clone()
    };
    for appearance in ThemeAppearance::ALL {
        let Some(half) = selection.halves.get(appearance) else {
            continue;
        };
        halves.set(appearance, (!removed(half)).then(|| half.to_string()));
    }
    ThemeSelection {
        theme: if removes_base {
            STANDARD_THEME_ID.to_string()
        } else {
            selection.theme.clone()
        },
        halves,
    }
}

/// The appearance the app shows: what the mode asks for, unless neither its half nor the base
/// theme has colors for it.
pub fn resolve_theme_appearance(
    selection: &ThemeSelection,
    mode: ThemePreference,
    system_dark: bool,
    custom_themes: &[ThemeDefinition],
) -> ThemeAppearance {
    let wanted = match mode {
        ThemePreference::System if system_dark => ThemeAppearance::Dark,
        ThemePreference::System | ThemePreference::Light => ThemeAppearance::Light,
        ThemePreference::Dark => ThemeAppearance::Dark,
    };
    if valid_halves(&selection.halves, custom_themes)
        .get(wanted)
        .is_some()
    {
        return wanted;
    }
    match find_theme(&selection.theme, custom_themes) {
        Some(base) if theme_colors_for(base, wanted).is_none() => base.appearance,
        _ => wanted,
    }
}

/// The theme painting an appearance, with its colors for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedTheme<'a> {
    pub theme: &'a ThemeDefinition,
    pub colors: &'a ThemeColors,
}

pub fn resolve_theme_for<'a>(
    selection: &ThemeSelection,
    appearance: ThemeAppearance,
    custom_themes: &'a [ThemeDefinition],
) -> ResolvedTheme<'a> {
    let halves = valid_halves(&selection.halves, custom_themes);
    let id = halves.get(appearance).unwrap_or(&selection.theme);
    let theme = find_theme(id, custom_themes).unwrap_or(&built_in_themes()[0]);
    let colors = theme_colors_for(theme, appearance).unwrap_or(&theme.colors);
    ResolvedTheme { theme, colors }
}

fn default_canvas(appearance: ThemeAppearance) -> Rgb {
    match appearance {
        ThemeAppearance::Light => Rgb::new(250.0, 250.0, 250.0),
        ThemeAppearance::Dark => Rgb::new(24.0, 24.0, 27.0),
    }
}

const DEFAULT_ACCENT: Rgb = Rgb::new(79.0, 70.0, 229.0);

/// Every role from a background and an accent, the editor's simple mode.
pub fn derive_theme_colors(
    appearance: ThemeAppearance,
    background: &str,
    accent_value: &str,
) -> ThemeColors {
    let canvas_rgb = color_to_rgb(background, default_canvas(appearance));
    let accent_rgb = color_to_rgb(accent_value, DEFAULT_ACCENT);
    let canvas = rgb_to_oklch(canvas_rgb);
    let accent = rgb_to_oklch(accent_rgb);
    let dark = is_dark_rgb(canvas_rgb);
    let hue = if accent.c < 0.02 { canvas.h } else { accent.h };
    let tint = 0.045f64.min(0.008f64.max(accent.c * 0.22));
    let step = if dark { 1.0 } else { -1.0 };
    let surface_at = |delta: f64, chroma: f64| Oklch {
        l: 0.99f64.min(0.05f64.max(canvas.l + step * delta)),
        c: chroma,
        h: hue,
    };
    let pick = |dark_value: f64, light_value: f64| if dark { dark_value } else { light_value };
    let direction = if dark {
        LightnessDirection::Lighter
    } else {
        LightnessDirection::Darker
    };
    let color = |value: Oklch| oklch_to_color(value, 1.0);
    let text_base = Oklch {
        l: pick(0.96, 0.18),
        c: 0.03f64.min(accent.c * 0.2),
        h: hue,
    };
    let text = solve_lightness(text_base, &[canvas_rgb], 7.0, direction);
    let text_rgb = oklch_to_rgb(text, true);
    let foreground_on = |surface: Oklch| {
        color(solve_lightness(
            text_base,
            &[oklch_to_rgb(surface, true)],
            4.6,
            direction,
        ))
    };
    let card = if dark {
        surface_at(0.06, tint)
    } else {
        Oklch {
            l: 0.998f64.min(canvas.l + 0.007),
            c: tint * 0.6,
            h: hue,
        }
    };
    let secondary = surface_at(pick(0.12, 0.03), 0.05f64.min(accent.c * 0.3));
    let highlight = surface_at(pick(0.13, 0.04), 0.06f64.min(accent.c * 0.4));
    let sidebar = surface_at(pick(0.05, 0.016), tint * 1.4);
    let sidebar_accent = surface_at(pick(0.13, 0.05), 0.06f64.min(accent.c * 0.4));
    let border = surface_at(pick(0.12, 0.075), 0.04f64.min(accent.c * 0.25));
    let input = surface_at(pick(0.16, 0.075), 0.04f64.min(accent.c * 0.25));
    let accent_color = rgb_to_color(accent_rgb, 1.0);
    let accent_foreground = rgb_to_color(readable_foreground(accent_rgb), 1.0);
    let ring = rgb_to_color(mix_rgb(accent_rgb, canvas_rgb, 0.35), 1.0);
    let link = readable_on(&accent_color, &rgb_to_color(canvas_rgb, 1.0), 4.5);
    let muted = rgb_to_color(softened_text(canvas_rgb, text_rgb, pick(5.0, 4.7)), 1.0);
    let header_glow = oklch_to_color(
        Oklch {
            l: 0.85,
            c: 0.06f64.min(accent.c),
            h: hue,
        },
        pick(0.08, 0.35),
    );
    let terminal_selection = oklch_to_color(
        Oklch {
            l: pick(0.6, 0.7),
            c: 0.08f64.min(accent.c),
            h: hue,
        },
        pick(0.45, 0.35),
    );
    let mut colors = standard_colors(appearance).clone();
    let derived = [
        ("background", rgb_to_color(canvas_rgb, 1.0)),
        ("foreground", rgb_to_color(text_rgb, 1.0)),
        ("card", color(card)),
        ("cardForeground", foreground_on(card)),
        ("popover", color(card)),
        ("popoverForeground", foreground_on(card)),
        ("primary", accent_color.clone()),
        ("primaryForeground", accent_foreground.clone()),
        ("secondary", color(secondary)),
        ("secondaryForeground", foreground_on(secondary)),
        ("muted", color(secondary)),
        ("mutedForeground", muted),
        ("accent", color(highlight)),
        ("accentForeground", foreground_on(highlight)),
        ("border", color(border)),
        ("input", color(input)),
        ("ring", ring.clone()),
        ("sidebar", color(sidebar)),
        ("sidebarForeground", foreground_on(sidebar)),
        ("sidebarPrimary", accent_color),
        ("sidebarPrimaryForeground", accent_foreground),
        ("sidebarAccent", color(sidebar_accent)),
        ("sidebarAccentForeground", foreground_on(sidebar_accent)),
        ("sidebarBorder", color(border)),
        ("sidebarRing", ring),
        ("header", color(surface_at(pick(0.045, 0.025), tint * 1.2))),
        ("headerGlow", header_glow),
        ("link", link.clone()),
        ("send", link),
        ("terminalSelection", terminal_selection),
        ("scrollbar", rgb_to_color(text_rgb, pick(0.16, 0.18))),
        ("scrollbarHover", rgb_to_color(text_rgb, pick(0.3, 0.34))),
    ];
    for (role, value) in derived {
        colors.set(role, value);
    }
    colors
}

/// Sets one color in the editor's advanced mode, and the roles that pair with it: text that sits
/// on a surface follows it so it stays readable.
pub fn update_color_family(colors: &ThemeColors, role: &str, value: &str) -> ThemeColors {
    let mut next = colors.clone();
    let Some(parsed) = parse_color(value) else {
        next.set(role, value);
        return next;
    };
    let normalized = oklch_to_color(parsed.color, parsed.alpha);
    let canvas = color_to_rgb(colors.get("background"), WHITE);
    let selected = mix_rgb(canvas, oklch_to_rgb(parsed.color, true), parsed.alpha);
    let foreground_on = || rgb_to_color(readable_foreground(selected), 1.0);
    let text_on = || {
        if contrast_ratio(color_to_rgb(colors.get("foreground"), BLACK), selected) >= 4.5 {
            colors.get("foreground").to_string()
        } else {
            foreground_on()
        }
    };
    let family: Vec<(&str, String)> = match role {
        "card" | "popover" => vec![
            ("card", normalized.clone()),
            ("popover", normalized),
            ("cardForeground", text_on()),
            ("popoverForeground", text_on()),
        ],
        "foreground" => vec![
            ("foreground", normalized.clone()),
            ("cardForeground", normalized.clone()),
            ("popoverForeground", normalized),
        ],
        "border" => vec![
            ("border", normalized.clone()),
            ("sidebarBorder", normalized),
        ],
        "secondary" => vec![
            ("secondary", normalized.clone()),
            ("muted", normalized),
            ("secondaryForeground", text_on()),
        ],
        "accent" => vec![("accent", normalized), ("accentForeground", text_on())],
        "primary" => {
            let ring = rgb_to_color(mix_rgb(selected, canvas, 0.35), 1.0);
            vec![
                ("primary", normalized.clone()),
                ("primaryForeground", foreground_on()),
                ("ring", ring.clone()),
                ("sidebarPrimary", normalized),
                ("sidebarPrimaryForeground", foreground_on()),
                ("sidebarRing", ring),
            ]
        }
        "link" => vec![("link", normalized.clone()), ("send", normalized)],
        "sidebar" => vec![("sidebar", normalized), ("sidebarForeground", text_on())],
        "sidebarAccent" => vec![
            ("sidebarAccent", normalized),
            ("sidebarAccentForeground", text_on()),
        ],
        "scrollbar" => vec![
            ("scrollbar", normalized),
            (
                "scrollbarHover",
                oklch_to_color(parsed.color, (parsed.alpha * 1.8).min(1.0)),
            ),
        ],
        "destructive" => vec![
            ("destructive", normalized.clone()),
            ("statusError", normalized),
        ],
        _ => vec![(role, normalized)],
    };
    for (role, value) in family {
        next.set(role, value);
    }
    next
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;

    pub(crate) fn dark_only() -> ThemeDefinition {
        ThemeDefinition::new(
            "midnight",
            "Midnight",
            ThemeAppearance::Dark,
            derive_theme_colors(ThemeAppearance::Dark, "#101018", "#7c6cff"),
        )
    }

    pub(crate) fn light_only() -> ThemeDefinition {
        ThemeDefinition::new(
            "paper",
            "Paper",
            ThemeAppearance::Light,
            derive_theme_colors(ThemeAppearance::Light, "#fbf8f1", "#b4532a"),
        )
    }

    fn customs() -> Vec<ThemeDefinition> {
        vec![dark_only(), light_only()]
    }

    fn selection(theme: &str, light: Option<&str>, dark: Option<&str>) -> ThemeSelection {
        ThemeSelection {
            theme: theme.to_string(),
            halves: ThemeHalves {
                light: light.map(str::to_string),
                dark: dark.map(str::to_string),
            },
        }
    }

    fn css_block(css: &str, selector: &str) -> HashMap<String, String> {
        let start = css
            .find(&format!("{selector} {{"))
            .expect("selector in the stylesheet");
        let end = start + css[start..].find('}').expect("block end");
        let mut values = HashMap::new();
        for segment in css[start..end].split(';') {
            let mut from = 0;
            while let Some(offset) = segment[from..].find("--") {
                let name_start = from + offset;
                let name_end = segment[name_start + 2..]
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
                    .map_or(segment.len(), |index| name_start + 2 + index);
                if segment[name_end..].starts_with(':') && name_end < segment.len() {
                    values.insert(
                        segment[name_start..name_end].to_string(),
                        segment[name_end + 1..].trim().to_string(),
                    );
                    break;
                }
                from = name_start + 2;
            }
        }
        values
    }

    fn stylesheet() -> String {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/hoshi-ui/assets/globals.css"
        );
        std::fs::read_to_string(path)
            .expect("globals.css")
            .replace("\r\n", "\n")
    }

    fn role_of(variable: &str) -> Option<&'static str> {
        THEME_COLOR_ROLES
            .into_iter()
            .find(|role| format!("--{}", role_variable(role)) == variable)
    }

    #[test]
    fn the_standard_theme_mirrors_the_stylesheet() {
        let css = stylesheet();
        let light = css_block(&css, ":root,\n.light");
        let dark = css_block(&css, ".dark");
        for role in THEME_COLOR_ROLES {
            let variable = format!("--{}", role_variable(role));
            assert_eq!(
                standard_colors(ThemeAppearance::Light).get(role),
                light[&variable]
            );
            assert_eq!(
                standard_colors(ThemeAppearance::Dark).get(role),
                dark[&variable]
            );
        }
    }

    #[test]
    fn the_palettes_mirror_the_stylesheet() {
        let css = stylesheet();
        let light = css_block(&css, "\n[data-palette]");
        let dark = css_block(&css, ".dark[data-palette]");
        for id in BUILT_IN_PALETTE_IDS {
            let tint = palette_tint(id).unwrap();
            let resolve = |value: &str| {
                value
                    .replace("var(--palette-hue)", &tint.hue.to_string())
                    .replace("var(--palette-chroma)", &tint.chroma.to_string())
            };
            for (block, appearance) in [
                (&light, ThemeAppearance::Light),
                (&dark, ThemeAppearance::Dark),
            ] {
                let colors = palette_colors(id, appearance).unwrap();
                for (variable, value) in block {
                    if let Some(role) = role_of(variable) {
                        assert_eq!(colors.get(role), resolve(value), "{id} {variable}");
                    }
                }
            }
        }
    }

    #[test]
    fn role_variable_names_the_token() {
        assert_eq!(
            role_variable("sidebarPrimaryForeground"),
            "sidebar-primary-foreground"
        );
        assert_eq!(role_variable("background"), "background");
    }

    #[test]
    fn appearances_parse_and_flip() {
        assert_eq!(ThemeAppearance::parse("dark"), Some(ThemeAppearance::Dark));
        assert_eq!(ThemeAppearance::parse("Dark"), None);
        assert_eq!(
            other_appearance(ThemeAppearance::Light),
            ThemeAppearance::Dark
        );
        assert_eq!(ThemeAppearance::Light.as_str(), "light");
    }

    #[test]
    fn a_whole_theme_clears_the_mix() {
        assert_eq!(choose_theme("grove"), selection("grove", None, None));
    }

    #[test]
    fn an_orb_takes_one_half_and_leaves_the_other_to_the_base() {
        let chosen = choose_half(&choose_theme("grove"), ThemeAppearance::Dark, Some("iris"));
        assert_eq!(chosen, selection("grove", None, Some("iris")));
        assert_eq!(
            half_owners(&chosen, &[]),
            ThemeHalves {
                light: Some("grove".into()),
                dark: Some("iris".into())
            }
        );
    }

    #[test]
    fn the_standard_orb_moves_a_themed_base_to_the_other_half() {
        let chosen = choose_half(&choose_theme("ocean"), ThemeAppearance::Light, None);
        assert_eq!(chosen, selection(STANDARD_THEME_ID, None, Some("ocean")));
        assert_eq!(
            half_owners(&chosen, &[]),
            ThemeHalves {
                light: None,
                dark: Some("ocean".into())
            }
        );
    }

    #[test]
    fn the_standard_orb_on_a_standard_base_clears_that_half() {
        let mixed = selection(STANDARD_THEME_ID, Some("ember"), None);
        assert_eq!(
            choose_half(&mixed, ThemeAppearance::Light, None),
            selection(STANDARD_THEME_ID, None, None)
        );
    }

    #[test]
    fn a_single_appearance_card_takes_its_half_a_full_one_the_whole() {
        let base = choose_theme("grove");
        assert_eq!(
            choose_card(&base, &dark_only()),
            selection("grove", None, Some("midnight"))
        );
        let mut both = light_only();
        both.variants.dark = Some(dark_only().colors);
        assert_eq!(choose_card(&base, &both), selection("paper", None, None));
    }

    #[test]
    fn halves_naming_a_missing_theme_or_appearance_are_dropped() {
        let customs = customs();
        let halves = |light: &str, dark: &str| ThemeHalves {
            light: Some(light.into()),
            dark: Some(dark.into()),
        };
        assert_eq!(
            valid_halves(&halves("midnight", "gone"), &customs),
            ThemeHalves::default()
        );
        assert_eq!(
            valid_halves(&halves("paper", "midnight"), &customs),
            halves("paper", "midnight")
        );
    }

    #[test]
    fn removing_themes_hands_their_halves_back() {
        assert_eq!(
            selection_without(&selection("paper", None, Some("midnight")), &["paper"]),
            selection(STANDARD_THEME_ID, None, Some("midnight"))
        );
        assert_eq!(
            selection_without(&selection("grove", None, Some("midnight")), &["midnight"]),
            selection("grove", None, None)
        );
    }

    #[test]
    fn resolve_theme_appearance_follows_the_mode() {
        let chosen = choose_theme("grove");
        assert_eq!(
            resolve_theme_appearance(&chosen, ThemePreference::System, true, &[]),
            ThemeAppearance::Dark
        );
        assert_eq!(
            resolve_theme_appearance(&chosen, ThemePreference::Light, true, &[]),
            ThemeAppearance::Light
        );
    }

    #[test]
    fn a_single_appearance_base_keeps_its_own_appearance() {
        assert_eq!(
            resolve_theme_appearance(
                &choose_theme("midnight"),
                ThemePreference::Light,
                false,
                &customs()
            ),
            ThemeAppearance::Dark
        );
    }

    #[test]
    fn a_half_for_the_wanted_appearance_wins_over_the_base() {
        let customs = customs();
        let chosen = selection("midnight", Some("paper"), None);
        assert_eq!(
            resolve_theme_appearance(&chosen, ThemePreference::Light, false, &customs),
            ThemeAppearance::Light
        );
        assert_eq!(
            resolve_theme_for(&chosen, ThemeAppearance::Light, &customs)
                .theme
                .id,
            "paper"
        );
        assert_eq!(
            resolve_theme_for(&chosen, ThemeAppearance::Dark, &customs)
                .theme
                .id,
            "midnight"
        );
    }

    #[test]
    fn theme_files_round_trip_through_serialize_theme_file() {
        let text = serialize_theme_file(&light_only());
        let parsed = parse_theme_file(&serde_json::from_str(&text).unwrap()).unwrap();
        assert_eq!(parsed.id, "paper");
        assert_eq!(parsed.colors, light_only().colors);
    }

    #[test]
    fn serialize_theme_file_writes_like_json_stringify() {
        let mut theme = built_in_themes()[1].clone();
        theme.id = "pack".into();
        theme.label = "Pack \"one\"".into();
        theme.collection = Some(ThemeCollection {
            id: "open-vsx:a.b".into(),
            label: "Pack".into(),
        });
        theme.managed = true;
        let text = serialize_theme_file(&theme);
        assert!(text.starts_with("{\n  \"version\": 1,\n  \"id\": \"pack\",\n  \"name\": \"Pack \\\"one\\\"\",\n  \"appearance\": \"light\",\n  \"colors\": {\n    \"background\": \"oklch(0.988 0.004 158)\",\n"));
        assert!(text.contains("\n  },\n  \"variants\": {\n    \"dark\": {\n      \"background\": \"oklch(0.16 0.01 158)\",\n"));
        assert!(text.ends_with(
            "\n    }\n  },\n  \"collection\": {\n    \"id\": \"open-vsx:a.b\",\n    \"label\": \"Pack\"\n  },\n  \"managed\": true\n}\n"
        ));
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value, theme_file_json(&theme));
        assert_eq!(
            serde_json::to_string_pretty(&value).unwrap().len() + 1,
            text.len()
        );
    }

    #[test]
    fn theme_files_fill_missing_roles_from_the_standard_theme() {
        let theme = parse_theme_file(&json!({
            "version": 1,
            "name": "Tiny",
            "appearance": "dark",
            "colors": { "background": "#000000" }
        }))
        .unwrap();
        assert_eq!(theme.id, "tiny");
        assert_eq!(
            theme.colors.get("foreground"),
            standard_colors(ThemeAppearance::Dark).get("foreground")
        );
    }

    #[test]
    fn theme_files_reject_what_they_cannot_use() {
        let code = |value: Value| parse_theme_file(&value).unwrap_err().code;
        assert_eq!(code(json!([])), ThemeErrorCode::NotAnObject);
        assert_eq!(
            code(json!({ "version": 2 })),
            ThemeErrorCode::UnsupportedVersion
        );
        assert_eq!(
            code(json!({ "version": 1, "name": "Grove", "appearance": "light", "colors": {} })),
            ThemeErrorCode::ReservedId
        );
        assert_eq!(
            code(
                json!({ "version": 1, "name": "X", "appearance": "light", "colors": { "glow": "#fff" } })
            ),
            ThemeErrorCode::UnknownRole
        );
        assert_eq!(
            code(
                json!({ "version": 1, "name": "X", "appearance": "light", "colors": { "background": "blue" } })
            ),
            ThemeErrorCode::InvalidColor
        );
        assert_eq!(
            code(json!({ "version": 1, "name": "X", "appearance": "light", "colors": {} })),
            ThemeErrorCode::NoColors
        );
        assert_eq!(
            code(json!({
                "version": 1, "name": "X", "appearance": "light",
                "colors": { "background": "#fff" }, "variants": { "light": { "background": "#fff" } }
            })),
            ThemeErrorCode::VariantRepeatsAppearance
        );
    }

    #[test]
    fn stored_themes_skip_unknown_roles_instead_of_failing() {
        let light = light_only();
        let mut stored = light.to_json();
        stored["colors"]["retired"] = json!("#fff");
        assert_eq!(parse_stored_theme(&stored).unwrap().colors, light.colors);
        stored["id"] = json!("grove");
        assert_eq!(parse_stored_theme(&stored), None);
    }

    #[test]
    fn stored_themes_round_trip_through_to_json() {
        let mut theme = light_only();
        theme.variants.dark = Some(dark_only().colors);
        theme.collection = Some(ThemeCollection {
            id: "open-vsx:a.b".into(),
            label: "Pack".into(),
        });
        theme.managed = true;
        assert_eq!(parse_stored_theme(&theme.to_json()), Some(theme.clone()));
        assert_eq!(
            parse_stored_themes(&json!([theme.to_json(), theme.to_json(), { "id": "x" }])),
            vec![theme]
        );
    }

    #[test]
    fn the_library_refuses_a_second_theme_with_the_same_id() {
        let error = install_theme(&customs(), dark_only()).unwrap_err();
        assert_eq!(error.code, ThemeErrorCode::AlreadyInstalled);
        assert_eq!(error.to_string(), "already_installed: Midnight");
    }

    #[test]
    fn the_library_replaces_a_collection_in_place() {
        let collection = ThemeCollection {
            id: "open-vsx:a.b".into(),
            label: "Pack".into(),
        };
        let old = ThemeDefinition {
            id: "old".into(),
            collection: Some(collection.clone()),
            ..dark_only()
        };
        let next = ThemeDefinition {
            id: "new".into(),
            collection: Some(collection.clone()),
            ..dark_only()
        };
        let replaced = replace_collection(
            &[light_only(), old, dark_only()],
            &collection.id,
            vec![next],
        )
        .unwrap();
        let ids: Vec<&str> = replaced.iter().map(|theme| theme.id.as_str()).collect();
        assert_eq!(ids, ["paper", "new", "midnight"]);
    }

    #[test]
    fn theme_id_from_name_slugs_names_and_hashes_the_ones_without_ascii() {
        assert_eq!(theme_id_from_name("  Tokyo Night! "), "tokyo-night");
        let hashed = theme_id_from_name("夜");
        let suffix = hashed.strip_prefix("theme-").unwrap();
        assert!(
            !suffix.is_empty()
                && suffix
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        );
        assert_eq!(theme_id_from_name("   "), "custom-theme");
    }

    #[test]
    fn derive_theme_colors_keeps_text_readable_on_the_background() {
        for (appearance, background) in [
            (ThemeAppearance::Light, "#fdfcf8"),
            (ThemeAppearance::Dark, "#14121a"),
        ] {
            let colors = derive_theme_colors(appearance, background, "#2d7ff9");
            let ratio = contrast_ratio(
                color_to_rgb(colors.get("foreground"), WHITE),
                color_to_rgb(colors.get("background"), WHITE),
            );
            assert!(ratio >= 7.0);
        }
    }

    #[test]
    fn derived_colors_are_canonical() {
        let colors = derive_theme_colors(ThemeAppearance::Dark, "#101018", "#7c6cff");
        for (role, value) in colors.iter() {
            assert_eq!(canonical_color(value).as_deref(), Some(value), "{role}");
        }
    }

    #[test]
    fn the_accent_carries_its_foreground_and_rings_along() {
        let colors = update_color_family(
            standard_colors(ThemeAppearance::Light),
            "primary",
            "#ffd400",
        );
        assert_eq!(colors.get("sidebarPrimary"), colors.get("primary"));
        let ratio = contrast_ratio(
            color_to_rgb(colors.get("primaryForeground"), WHITE),
            color_to_rgb(colors.get("primary"), WHITE),
        );
        assert!(ratio >= 4.5);
    }
}
