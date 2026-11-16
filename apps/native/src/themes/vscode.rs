//! Themes from VS Code color themes and Open VSX extensions (`lib/vscode-theme.ts`).

use super::color::{
    Rgb, WHITE, canonical_color, color_to_hex, color_to_rgb, contrast_ratio, is_dark_rgb, mix_rgb,
    parse_float_prefix, readable_foreground, readable_on, rgb_to_color, rgb_to_hex,
};
use super::model::{
    THEME_FILE_VERSION, THEME_NAME_MAX, ThemeAppearance, ThemeCollection, ThemeDefinition,
    ThemeError, ThemeErrorCode, derive_theme_colors, is_reserved_theme_id, parse_theme_file,
    standard_colors, theme_id_from_name, theme_modes, utf16_prefix,
};
use hoshi_core::{ExtensionTheme, ThemeExtension};
use serde_json::{Map, Value, json};

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rgba {
    r: f64,
    g: f64,
    b: f64,
    a: f64,
}

impl Rgba {
    fn rgb(self) -> Rgb {
        Rgb::new(self.r, self.g, self.b)
    }
}

fn skip_comma(rest: &[char]) -> bool {
    matches!(rest.iter().find(|c| !c.is_whitespace()), Some('}' | ']'))
}

/// Reads JSON with comments and trailing commas, as VS Code writes it.
pub fn parse_jsonc(source: &str) -> Result<Value, serde_json::Error> {
    let text: Vec<char> = source
        .strip_prefix('\u{feff}')
        .unwrap_or(source)
        .chars()
        .collect();
    let mut stripped: Vec<char> = Vec::with_capacity(text.len());
    let mut in_string = false;
    let mut index = 0;
    while index < text.len() {
        let c = text[index];
        if in_string {
            stripped.push(c);
            if c == '\\' {
                if let Some(next) = text.get(index + 1) {
                    stripped.push(*next);
                }
                index += 1;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            stripped.push(c);
        } else if c == '/' && text.get(index + 1) == Some(&'/') {
            while index < text.len() && text[index] != '\n' {
                index += 1;
            }
            stripped.push('\n');
        } else if c == '/' && text.get(index + 1) == Some(&'*') {
            let end = (index + 2..text.len().saturating_sub(1))
                .find(|at| text[*at] == '*' && text[at + 1] == '/');
            index = end.map_or(text.len(), |end| end + 1);
            stripped.push(' ');
        } else {
            stripped.push(c);
        }
        index += 1;
    }
    let mut result = String::with_capacity(stripped.len());
    in_string = false;
    let mut index = 0;
    while index < stripped.len() {
        let c = stripped[index];
        if in_string {
            result.push(c);
            if c == '\\' {
                if let Some(next) = stripped.get(index + 1) {
                    result.push(*next);
                }
                index += 1;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            result.push(c);
        } else if c != ',' || !skip_comma(&stripped[index + 1..]) {
            result.push(c);
        }
        index += 1;
    }
    serde_json::from_str(&result)
}

fn decode_gamma(value: f64) -> f64 {
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn encode_gamma(value: f64) -> f64 {
    let clamped = value.clamp(0.0, 1.0);
    if clamped <= 0.0031308 {
        clamped * 12.92
    } else {
        1.055 * clamped.powf(1.0 / 2.4) - 0.055
    }
}

fn parse_fraction(part: &str) -> Option<f64> {
    let value = parse_float_prefix(part)?;
    let value = if part.ends_with('%') {
        value / 100.0
    } else {
        value
    };
    value.is_finite().then_some(value)
}

fn parse_color_function(value: &str) -> Option<Rgba> {
    let inner = value
        .get(..6)
        .filter(|prefix| prefix.eq_ignore_ascii_case("color("))
        .and(value.get(6..))?
        .strip_suffix(')')?;
    if inner.contains(')') {
        return None;
    }
    let inner = inner.trim_start();
    let space_end = inner.find(char::is_whitespace)?;
    let space = inner[..space_end].to_ascii_lowercase();
    if space != "display-p3" && space != "srgb" {
        return None;
    }
    let arguments = inner[space_end..].trim_start();
    if arguments.is_empty() {
        return None;
    }
    let mut parts = arguments.split('/');
    let channel_part = parts.next().unwrap_or("");
    let alpha_part = parts.next();
    let channels = channel_part
        .trim()
        .split(char::is_whitespace)
        .filter(|part| !part.is_empty())
        .map(parse_fraction)
        .collect::<Option<Vec<f64>>>()?;
    let [red, green, blue] = channels[..] else {
        return None;
    };
    let alpha = match alpha_part {
        None => 1.0,
        Some(alpha) => parse_fraction(alpha.trim())?,
    };
    let a = alpha.clamp(0.0, 1.0);
    if space == "srgb" {
        return Some(Rgba {
            r: red * 255.0,
            g: green * 255.0,
            b: blue * 255.0,
            a,
        });
    }
    let (lr, lg, lb) = (decode_gamma(red), decode_gamma(green), decode_gamma(blue));
    Some(Rgba {
        r: encode_gamma(1.2249401762805 * lr - 0.2249401762805 * lg) * 255.0,
        g: encode_gamma(-0.042056961239 * lr + 1.042056961239 * lg) * 255.0,
        b: encode_gamma(-0.0196375547643 * lr - 0.0786360655012 * lg + 1.0982736202656 * lb)
            * 255.0,
        a,
    })
}

fn parse_vscode_color(value: &Value) -> Option<Rgba> {
    let trimmed = value.as_str()?.trim();
    if trimmed.starts_with("color(") {
        return parse_color_function(trimmed);
    }
    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if !matches!(hex.len(), 3 | 4 | 6 | 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let expand = |part: &str| {
        let digits = if part.len() == 1 {
            part.repeat(2)
        } else {
            part.to_string()
        };
        f64::from(u8::from_str_radix(&digits, 16).unwrap_or(0))
    };
    if hex.len() <= 4 {
        return Some(Rgba {
            r: expand(&hex[0..1]),
            g: expand(&hex[1..2]),
            b: expand(&hex[2..3]),
            a: if hex.len() == 4 {
                expand(&hex[3..4]) / 255.0
            } else {
                1.0
            },
        });
    }
    Some(Rgba {
        r: expand(&hex[0..2]),
        g: expand(&hex[2..4]),
        b: expand(&hex[4..6]),
        a: if hex.len() == 8 {
            expand(&hex[6..8]) / 255.0
        } else {
            1.0
        },
    })
}

/// A translucent VS Code overlay composited onto its surface; hoshi's surfaces are opaque.
fn flatten_over(color: Rgba, base: Rgb) -> String {
    rgb_to_hex(if color.a >= 1.0 {
        color.rgb()
    } else {
        mix_rgb(base, color.rgb(), color.a)
    })
}

fn hex_rgb(value: &str) -> Rgb {
    color_to_rgb(value, WHITE)
}

pub fn is_vscode_theme_file(value: &Value) -> bool {
    let Some(record) = value.as_object() else {
        return false;
    };
    if record.get("version").and_then(Value::as_f64) == Some(THEME_FILE_VERSION as f64) {
        return false;
    }
    let workbench = record
        .get("colors")
        .and_then(Value::as_object)
        .is_some_and(|colors| colors.keys().any(|key| key.contains('.')));
    workbench || record.get("tokenColors").is_some_and(Value::is_array)
}

pub fn humanize_theme_name(raw: &str) -> String {
    let trimmed = raw.trim();
    let separator = |c: char| matches!(c, '-' | '_' | '.');
    if trimmed.contains(char::is_whitespace) || !trimmed.contains(separator) {
        return trimmed.to_string();
    }
    trimmed
        .split(separator)
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn resolve_name(record: &Map<String, Value>, fallback: &str) -> String {
    ["displayName", "name"]
        .into_iter()
        .filter_map(|key| record.get(key)?.as_str())
        .map(humanize_theme_name)
        .find(|humanized| !humanized.is_empty())
        .map_or_else(
            || fallback.to_string(),
            |humanized| utf16_prefix(&humanized, THEME_NAME_MAX).to_string(),
        )
}

fn resolve_appearance(record: &Map<String, Value>, canvas: Rgb) -> ThemeAppearance {
    let kind = record
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_lowercase);
    match kind.as_deref() {
        Some("light" | "hc-light") => ThemeAppearance::Light,
        Some("dark" | "hc-black") => ThemeAppearance::Dark,
        _ if is_dark_rgb(canvas) => ThemeAppearance::Dark,
        _ => ThemeAppearance::Light,
    }
}

/// A hoshi theme from a VS Code color theme's workbench colors.
pub fn parse_vscode_theme_file(
    value: &Value,
    fallback_name: &str,
) -> Result<ThemeDefinition, ThemeError> {
    let record = value
        .as_object()
        .ok_or(ThemeError::new(ThemeErrorCode::NotAnObject))?;
    let empty = Map::new();
    let colors = record
        .get("colors")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    let pick = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| colors.get(*key).and_then(parse_vscode_color))
    };
    let solid_over = |base: Rgb, keys: &[&str]| pick(keys).map(|parsed| flatten_over(parsed, base));
    let canvas = pick(&["editor.background", "editorPane.background"])
        .ok_or(ThemeError::new(ThemeErrorCode::VscodeNoBackground))?
        .rgb();
    let appearance = resolve_appearance(record, canvas);
    let canvas_hex = rgb_to_hex(canvas);
    let raised_hex = solid_over(canvas, &["editorWidget.background", "dropdown.background"]);
    let stands_apart =
        |first: &str, second: &str| contrast_ratio(hex_rgb(first), hex_rgb(second)) >= 1.1;
    let stands_out = |candidate: &str| {
        stands_apart(candidate, &canvas_hex)
            && raised_hex
                .as_deref()
                .is_none_or(|raised| stands_apart(candidate, raised))
    };

    let accent_hex = [
        "focusBorder",
        "button.background",
        "textLink.foreground",
        "activityBarBadge.background",
        "progressBar.background",
        "badge.background",
    ]
    .into_iter()
    .filter_map(|key| colors.get(key).and_then(parse_vscode_color))
    .map(|candidate| flatten_over(candidate, canvas))
    .find(|candidate| stands_out(candidate))
    .unwrap_or_else(|| {
        let standard = color_to_hex(standard_colors(appearance).get("primary")).unwrap_or_default();
        [standard.as_str(), "#ffffff", "#000000"]
            .into_iter()
            .find(|candidate| stands_out(candidate))
            .unwrap_or(&standard)
            .to_string()
    });
    let accent_rgb = hex_rgb(&accent_hex);
    let derived = derive_theme_colors(
        appearance,
        &canvas_hex,
        &rgb_to_hex(mix_rgb(canvas, accent_rgb, 0.2)),
    );
    let readable_on_surface = |surface: &str, fallback: &str, keys: &[&str]| {
        let surface_rgb = hex_rgb(surface);
        let readable = |candidate: &str| contrast_ratio(hex_rgb(candidate), surface_rgb) >= 4.5;
        if let Some(specified) = solid_over(surface_rgb, keys)
            && readable(&specified)
        {
            return specified;
        }
        if readable(fallback) {
            return fallback.to_string();
        }
        rgb_to_color(readable_foreground(surface_rgb), 1.0)
    };
    let status_color = |fallback: &str, keys: &[&str]| {
        let specified =
            solid_over(canvas, keys).filter(|specified| stands_apart(specified, &canvas_hex));
        readable_on(specified.as_deref().unwrap_or(fallback), &canvas_hex, 4.5)
    };
    let translucent = |fallback: &str, keys: &[&str]| {
        keys.iter()
            .find_map(|key| colors.get(*key)?.as_str().and_then(canonical_color))
            .unwrap_or_else(|| fallback.to_string())
    };

    let card_hex = solid_over(canvas, &["editorWidget.background"])
        .unwrap_or_else(|| derived.get("card").to_string());
    let popover_hex = solid_over(
        canvas,
        &[
            "menu.background",
            "quickInput.background",
            "dropdown.background",
        ],
    )
    .unwrap_or_else(|| card_hex.clone());
    let send_hex = solid_over(canvas, &["button.background"])
        .filter(|button| stands_apart(button, &canvas_hex))
        .unwrap_or_else(|| accent_hex.clone());
    let standard_input = standard_colors(appearance).get("input");
    let input_hex = ["input.background", "input.border"]
        .into_iter()
        .filter_map(|key| solid_over(canvas, &[key]))
        .find(|candidate| stands_apart(candidate, &canvas_hex))
        .unwrap_or_else(|| {
            [
                derived.get("input"),
                derived.get("card"),
                standard_input,
                "#000000",
                "#ffffff",
                "#808080",
            ]
            .into_iter()
            .find(|candidate| stands_apart(candidate, &canvas_hex))
            .unwrap_or("#808080")
            .to_string()
        });
    let sidebar_hex = solid_over(canvas, &["sideBar.background", "activityBar.background"])
        .unwrap_or_else(|| derived.get("sidebar").to_string());
    let sidebar = hex_rgb(&sidebar_hex);
    let highlight_hex = solid_over(
        canvas,
        &["list.activeSelectionBackground", "list.hoverBackground"],
    )
    .unwrap_or_else(|| derived.get("accent").to_string());
    let sidebar_selection_hex = solid_over(
        sidebar,
        &[
            "list.activeSelectionBackground",
            "list.inactiveSelectionBackground",
            "list.hoverBackground",
        ],
    )
    .unwrap_or_else(|| derived.get("sidebarAccent").to_string());
    let foreground = readable_on_surface(
        &canvas_hex,
        derived.get("foreground"),
        &["editor.foreground", "foreground"],
    );
    let ring = rgb_to_color(mix_rgb(accent_rgb, canvas, 0.35), 1.0);
    let primary_foreground = readable_on_surface(
        &accent_hex,
        derived.get("primaryForeground"),
        &["button.foreground"],
    );
    let error = status_color(
        derived.get("destructive"),
        &["editorError.foreground", "errorForeground"],
    );

    let overrides = [
        ("background", canvas_hex.clone()),
        ("foreground", foreground.clone()),
        ("card", card_hex.clone()),
        (
            "cardForeground",
            readable_on_surface(&card_hex, &foreground, &["editor.foreground"]),
        ),
        ("popover", popover_hex.clone()),
        (
            "popoverForeground",
            readable_on_surface(&popover_hex, &foreground, &["editor.foreground"]),
        ),
        ("primary", accent_hex.clone()),
        ("primaryForeground", primary_foreground.clone()),
        (
            "mutedForeground",
            readable_on_surface(
                &canvas_hex,
                derived.get("mutedForeground"),
                &["descriptionForeground", "disabledForeground"],
            ),
        ),
        ("accent", highlight_hex.clone()),
        (
            "accentForeground",
            readable_on_surface(&highlight_hex, &foreground, &[]),
        ),
        ("destructive", error.clone()),
        ("statusError", error),
        (
            "statusWaiting",
            status_color(derived.get("statusWaiting"), &["editorWarning.foreground"]),
        ),
        (
            "border",
            solid_over(
                canvas,
                &["panel.border", "editorGroup.border", "contrastBorder"],
            )
            .unwrap_or_else(|| derived.get("border").to_string()),
        ),
        ("input", input_hex),
        ("ring", ring.clone()),
        ("sidebar", sidebar_hex.clone()),
        (
            "sidebarForeground",
            readable_on_surface(
                &sidebar_hex,
                derived.get("sidebarForeground"),
                &["sideBar.foreground"],
            ),
        ),
        ("sidebarPrimary", accent_hex.clone()),
        ("sidebarPrimaryForeground", primary_foreground),
        ("sidebarAccent", sidebar_selection_hex.clone()),
        (
            "sidebarAccentForeground",
            readable_on_surface(&sidebar_selection_hex, &foreground, &[]),
        ),
        (
            "sidebarBorder",
            solid_over(sidebar, &["sideBar.border"])
                .unwrap_or_else(|| derived.get("sidebarBorder").to_string()),
        ),
        ("sidebarRing", ring),
        (
            "link",
            readable_on(
                &solid_over(canvas, &["textLink.foreground"]).unwrap_or_else(|| accent_hex.clone()),
                &canvas_hex,
                4.5,
            ),
        ),
        ("send", send_hex),
        (
            "terminalSelection",
            translucent(
                derived.get("terminalSelection"),
                &["terminal.selectionBackground", "editor.selectionBackground"],
            ),
        ),
        (
            "scrollbar",
            translucent(derived.get("scrollbar"), &["scrollbarSlider.background"]),
        ),
    ];
    let mut theme_colors = derived.clone();
    for (role, value) in overrides {
        theme_colors.set(role, value);
    }

    let name = resolve_name(record, fallback_name);
    let generated_id = theme_id_from_name(&name);
    let mut file = Map::new();
    file.insert("version".into(), json!(THEME_FILE_VERSION));
    if is_reserved_theme_id(&generated_id) {
        file.insert("id".into(), json!(format!("{generated_id}-vscode")));
    }
    file.insert("name".into(), json!(name));
    file.insert("appearance".into(), json!(appearance.as_str()));
    file.insert("colors".into(), theme_colors.to_json());
    parse_theme_file(&Value::Object(file))
}

/// A hoshi theme file or a VS Code color theme, from its JSON text.
pub fn parse_theme_import(
    source: &str,
    fallback_name: &str,
) -> Result<ThemeDefinition, ThemeError> {
    let value = parse_jsonc(source).map_err(|_| ThemeError::new(ThemeErrorCode::InvalidJson))?;
    if is_vscode_theme_file(&value) {
        parse_vscode_theme_file(&value, fallback_name)
    } else {
        parse_theme_file(&value)
    }
}

fn file_without_id(theme: &ThemeDefinition, name: &str) -> Map<String, Value> {
    let mut file = Map::new();
    file.insert("version".into(), json!(THEME_FILE_VERSION));
    file.insert("name".into(), json!(name));
    file.insert("appearance".into(), json!(theme.appearance.as_str()));
    file.insert("colors".into(), theme.colors.to_json());
    if !theme.variants.is_empty() {
        let mut variants = Map::new();
        for appearance in ThemeAppearance::ALL {
            if let Some(colors) = theme.variant(appearance) {
                variants.insert(appearance.as_str().into(), colors.to_json());
            }
        }
        file.insert("variants".into(), Value::Object(variants));
    }
    if theme.managed {
        file.insert("managed".into(), Value::Bool(true));
    }
    file
}

fn rename(theme: &ThemeDefinition, name: &str) -> Option<ThemeDefinition> {
    let file = file_without_id(theme, utf16_prefix(name, THEME_NAME_MAX));
    parse_theme_file(&Value::Object(file)).ok()
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The label without the words "light" and "dark", `\b(?:light|dark)\b` in any case.
fn strip_appearance(label: &str) -> String {
    let chars: Vec<char> = label.chars().collect();
    let mut out = String::with_capacity(label.len());
    let mut index = 0;
    while index < chars.len() {
        let boundary = index == 0 || !is_word_char(chars[index - 1]);
        let word = ["light", "dark"].into_iter().find(|word| {
            let end = index + word.len();
            boundary
                && end <= chars.len()
                && chars[index..end]
                    .iter()
                    .zip(word.chars())
                    .all(|(c, expected)| c.to_ascii_lowercase() == expected)
                && chars.get(end).is_none_or(|next| !is_word_char(*next))
        });
        match word {
            Some(word) => {
                out.push(' ');
                index += word.len();
            }
            None => {
                out.push(chars[index]);
                index += 1;
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Builds the id of a light and a dark theme joined into one.
pub type PairedId<'a> = &'a dyn Fn(&ThemeDefinition, &ThemeDefinition) -> String;

/// Joins a light and a dark theme that differ only by "Light" and "Dark" in their names.
pub fn pair_vscode_themes(
    themes: &[ThemeDefinition],
    paired_id: Option<PairedId<'_>>,
) -> Vec<ThemeDefinition> {
    struct Group<'a> {
        key: String,
        light: Vec<&'a ThemeDefinition>,
        dark: Vec<&'a ThemeDefinition>,
        order: usize,
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut entries: Vec<(usize, ThemeDefinition)> = Vec::new();
    for (order, theme) in themes.iter().enumerate() {
        let key = strip_appearance(&theme.label);
        if theme_modes(theme).len() != 1
            || key == theme.label
            || !key.chars().any(|c| c.is_ascii_alphanumeric())
        {
            entries.push((order, theme.clone()));
            continue;
        }
        let index = match groups.iter().position(|group| group.key == key) {
            Some(index) => index,
            None => {
                groups.push(Group {
                    key,
                    light: Vec::new(),
                    dark: Vec::new(),
                    order,
                });
                groups.len() - 1
            }
        };
        let group = &mut groups[index];
        match theme.appearance {
            ThemeAppearance::Light => group.light.push(theme),
            ThemeAppearance::Dark => group.dark.push(theme),
        }
    }
    for group in groups {
        if let ([light], [dark]) = (group.light.as_slice(), group.dark.as_slice()) {
            let mut file = Map::new();
            file.insert("version".into(), json!(THEME_FILE_VERSION));
            if let Some(paired_id) = paired_id {
                file.insert("id".into(), json!(paired_id(light, dark)));
            }
            file.insert("name".into(), json!(group.key));
            file.insert("appearance".into(), json!("light"));
            file.insert("colors".into(), light.colors.to_json());
            file.insert("variants".into(), json!({ "dark": dark.colors.to_json() }));
            if let Ok(paired) = parse_theme_file(&Value::Object(file)) {
                entries.push((group.order, paired));
                continue;
            }
        }
        for theme in group.light.into_iter().chain(group.dark) {
            entries.push((group.order, theme.clone()));
        }
    }
    entries.sort_by_key(|(order, _)| *order);
    entries.into_iter().map(|(_, theme)| theme).collect()
}

/// `name.json` → `name`, as `/\.[^.]+$/` strips it.
fn file_stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(index) if index + 1 < name.len() => &name[..index],
        _ => name,
    }
}

/// Themes that would share an id are renamed after their file, then numbered.
pub fn resolve_theme_label_collisions(
    entries: &[(ThemeDefinition, Option<&str>)],
) -> Vec<ThemeDefinition> {
    let count = |id: &str| entries.iter().filter(|(theme, _)| theme.id == id).count();
    let relabelled: Vec<ThemeDefinition> = entries
        .iter()
        .map(|(theme, source_name)| {
            if count(&theme.id) < 2 {
                return theme.clone();
            }
            source_name
                .map(file_stem)
                .filter(|stem| !stem.is_empty())
                .map(humanize_theme_name)
                .filter(|from_file| {
                    !from_file.is_empty() && from_file.to_lowercase() != theme.label.to_lowercase()
                })
                .and_then(|from_file| rename(theme, &from_file))
                .unwrap_or_else(|| theme.clone())
        })
        .collect();
    let mut seen: Vec<String> = Vec::new();
    relabelled
        .into_iter()
        .map(|theme| {
            if !seen.contains(&theme.id) {
                seen.push(theme.id.clone());
                return theme;
            }
            for suffix in 2..100 {
                if let Some(candidate) = rename(&theme, &format!("{} {suffix}", theme.label))
                    && !seen.contains(&candidate.id)
                {
                    seen.push(candidate.id.clone());
                    return candidate;
                }
            }
            theme
        })
        .collect()
}

fn short_hash(text: &str) -> String {
    let length = text.encode_utf16().count() as u32;
    let mut first: u32 = 0x811c_9dc5;
    let mut second: u32 = 0x0100_0193 ^ length;
    for code in text.encode_utf16() {
        let code = u32::from(code);
        first = (first ^ code).wrapping_mul(0x0100_0193);
        second = (second ^ code).wrapping_mul(0x5bd1_e995);
    }
    format!("{first:08x}{}", &format!("{second:08x}")[..4])
}

fn open_vsx_theme_id(extension_id: &str, source: &str) -> String {
    format!(
        "ovx-theme-{}",
        short_hash(&format!("{extension_id}:{source}"))
    )
}

/// The themes an Open VSX extension installs: each file read, paired and grouped.
pub fn extension_themes(
    extension: &ThemeExtension,
    files: &[ExtensionTheme],
) -> Result<Vec<ThemeDefinition>, ThemeError> {
    if files.is_empty() {
        return Err(ThemeError::new(ThemeErrorCode::NoCompatibleThemes));
    }
    let extension_id = extension.id.to_lowercase();
    let label = utf16_prefix(&extension.name, THEME_NAME_MAX);
    let mut parsed = Vec::with_capacity(files.len());
    for file in files {
        let mut value = Map::new();
        value.insert("colors".into(), json!(file.colors));
        value.insert("displayName".into(), json!(file.label));
        if let Some(ui_theme) = file.ui_theme.as_deref().filter(|kind| !kind.is_empty()) {
            value.insert("type".into(), json!(ui_theme));
        }
        let theme = parse_vscode_theme_file(&Value::Object(value), label)?;
        let source_name = file
            .source_path
            .rsplit('/')
            .next()
            .unwrap_or(&file.source_path);
        parsed.push((theme, Some(source_name)));
    }
    let resolved: Vec<ThemeDefinition> = resolve_theme_label_collisions(&parsed)
        .into_iter()
        .zip(files)
        .map(|(theme, file)| ThemeDefinition {
            id: open_vsx_theme_id(&extension_id, &file.source_path),
            ..theme
        })
        .collect();
    let paired_id = |light: &ThemeDefinition, dark: &ThemeDefinition| {
        let mut ids = [light.id.as_str(), dark.id.as_str()];
        ids.sort_unstable();
        open_vsx_theme_id(&extension_id, &ids.join(":"))
    };
    let paired = pair_vscode_themes(&resolved, Some(&paired_id));
    let collection = ThemeCollection {
        id: extension.collection_id.clone(),
        label: label.to_string(),
    };
    let unnamed: Vec<(ThemeDefinition, Option<&str>)> =
        paired.iter().map(|theme| (theme.clone(), None)).collect();
    Ok(resolve_theme_label_collisions(&unnamed)
        .into_iter()
        .zip(&paired)
        .map(|(theme, original)| ThemeDefinition {
            id: original.id.clone(),
            collection: Some(collection.clone()),
            ..theme
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn night() -> Value {
        json!({
            "name": "Night Owl",
            "type": "dark",
            "colors": {
                "editor.background": "#011627",
                "editor.foreground": "#d6deeb",
                "focusBorder": "#7e57c2",
                "sideBar.background": "#011627",
                "editorError.foreground": "#ef5350",
                "terminal.selectionBackground": "#1b90dd4d"
            },
            "tokenColors": []
        })
    }

    fn extension() -> ThemeExtension {
        ThemeExtension {
            id: "acme.owls".into(),
            collection_id: "open-vsx:acme.owls".into(),
            name: "Owls".into(),
            publisher: "acme".into(),
            description: String::new(),
            download_count: 1,
            icon_url: None,
            source_url: None,
            manifest_url: "https://open-vsx.org/m".into(),
            sha256_url: "https://open-vsx.org/s".into(),
            vsix_url: "https://open-vsx.org/v".into(),
            version: "1.0.0".into(),
            license: "MIT".into(),
        }
    }

    fn hex(value: &str) -> String {
        color_to_hex(value).unwrap()
    }

    #[test]
    fn parse_jsonc_ignores_comments_and_trailing_commas_but_not_inside_strings() {
        assert_eq!(
            parse_jsonc("{\n // hi\n \"a\": \"http://x\", /* b */ \"c\": [1,],\n}").unwrap(),
            json!({ "a": "http://x", "c": [1] })
        );
        assert_eq!(
            parse_jsonc("\u{feff}{ \"a\": \"q\\\"/*\", }").unwrap(),
            json!({ "a": "q\"/*" })
        );
        assert!(parse_jsonc("{ nope").is_err());
    }

    #[test]
    fn tells_vscode_themes_from_hoshi_theme_files() {
        assert!(is_vscode_theme_file(&night()));
        assert!(!is_vscode_theme_file(
            &json!({ "version": 1, "colors": { "editor.background": "#000" } })
        ));
    }

    #[test]
    fn maps_the_workbench_colors_onto_the_roles() {
        let theme = parse_vscode_theme_file(&night(), "VS Code theme").unwrap();
        assert_eq!(theme.label, "Night Owl");
        assert_eq!(theme.id, "night-owl");
        assert_eq!(theme.appearance, ThemeAppearance::Dark);
        assert_eq!(theme_modes(&theme), [ThemeAppearance::Dark]);
        assert_eq!(hex(theme.colors.get("background")), "#011627");
        assert_eq!(hex(theme.colors.get("primary")), "#7e57c2");
        assert_eq!(hex(theme.colors.get("foreground")), "#d6deeb");
        assert!(theme.colors.get("terminalSelection").ends_with("/ 0.302)"));
    }

    #[test]
    fn needs_an_editor_background() {
        let error = parse_vscode_theme_file(&json!({ "colors": { "foreground": "#fff" } }), "x")
            .unwrap_err();
        assert_eq!(error.code, ThemeErrorCode::VscodeNoBackground);
    }

    #[test]
    fn guesses_the_appearance_from_the_background() {
        let theme = parse_vscode_theme_file(
            &json!({ "colors": { "editor.background": "#fdf6e3" } }),
            "Solarized",
        )
        .unwrap();
        assert_eq!(theme.appearance, ThemeAppearance::Light);
        assert_eq!(theme.label, "Solarized");
    }

    #[test]
    fn reads_display_p3_and_srgb_color_functions() {
        let srgb = parse_vscode_color(&json!("color(srgb 1 0 0 / 50%)")).unwrap();
        assert_eq!((srgb.r, srgb.g, srgb.b, srgb.a), (255.0, 0.0, 0.0, 0.5));
        let p3 = parse_vscode_color(&json!("color(display-p3 0 0 1)")).unwrap();
        assert!(p3.b > 254.0 && p3.r < 1.0 && p3.a == 1.0);
        assert_eq!(parse_vscode_color(&json!("color(rec2020 1 0 0)")), None);
        assert_eq!(parse_vscode_color(&json!("color(srgb 1 0)")), None);
    }

    #[test]
    fn reads_either_format_from_text() {
        let text = serde_json::to_string(&night()).unwrap();
        assert_eq!(parse_theme_import(&text, "x").unwrap().id, "night-owl");
        let mine = r##"{ "version": 1, "name": "Mine", "appearance": "light", "colors": { "background": "#fff" } }"##;
        assert_eq!(parse_theme_import(mine, "x").unwrap().id, "mine");
    }

    #[test]
    fn reports_text_that_is_not_json() {
        assert_eq!(
            parse_theme_import("{ nope", "x").unwrap_err().code,
            ThemeErrorCode::InvalidJson
        );
    }

    #[test]
    fn joins_a_light_and_a_dark_theme_of_the_same_name() {
        let mut dark_file = night();
        dark_file["name"] = json!("Owl Dark");
        let dark = parse_vscode_theme_file(&dark_file, "x").unwrap();
        let light = parse_vscode_theme_file(
            &json!({ "name": "Owl Light", "type": "light", "colors": { "editor.background": "#fbfbfb" } }),
            "x",
        )
        .unwrap();
        let paired = pair_vscode_themes(&[dark, light], None);
        assert_eq!(paired.len(), 1);
        assert_eq!(paired[0].label, "Owl");
        assert_eq!(
            theme_modes(&paired[0]),
            [ThemeAppearance::Light, ThemeAppearance::Dark]
        );
    }

    #[test]
    fn strips_appearance_words_only_as_whole_words() {
        assert_eq!(strip_appearance("Owl  Dark"), "Owl");
        assert_eq!(strip_appearance("LIGHT owl"), "owl");
        assert_eq!(strip_appearance("Darkness Lightly"), "Darkness Lightly");
        assert_eq!(strip_appearance("owl-dark"), "owl-");
    }

    #[test]
    fn renames_after_the_file_then_numbers() {
        let theme = parse_vscode_theme_file(&night(), "x").unwrap();
        let resolved = resolve_theme_label_collisions(&[
            (theme.clone(), Some("night-owl.json")),
            (theme.clone(), Some("night-owl-noitalic.json")),
            (theme, None),
        ]);
        let labels: Vec<&str> = resolved.iter().map(|theme| theme.label.as_str()).collect();
        assert_eq!(labels, ["Night Owl", "Night Owl Noitalic", "Night Owl 2"]);
    }

    #[test]
    fn gives_each_theme_a_stable_id_inside_the_collection() {
        let colors: BTreeMap<String, String> = night()["colors"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(key, value)| (key.clone(), value.as_str().unwrap().to_string()))
            .collect();
        let files = [ExtensionTheme {
            label: "Owl Night".into(),
            ui_theme: Some("dark".into()),
            source_path: "extension/themes/night.json".into(),
            colors,
        }];
        let themes = extension_themes(&extension(), &files).unwrap();
        let id = themes[0].id.strip_prefix("ovx-theme-").unwrap();
        assert!(
            id.len() == 12
                && id
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
        assert_eq!(
            extension_themes(&extension(), &files).unwrap()[0].id,
            themes[0].id
        );
        assert_eq!(
            themes[0].collection,
            Some(ThemeCollection {
                id: "open-vsx:acme.owls".into(),
                label: "Owls".into()
            })
        );
        assert_eq!(
            extension_themes(&extension(), &[]).unwrap_err().code,
            ThemeErrorCode::NoCompatibleThemes
        );
    }

    #[test]
    fn short_hash_matches_the_web() {
        assert_eq!(short_hash(""), "811c9dc50100");
    }

    #[test]
    fn turns_file_names_into_labels() {
        assert_eq!(
            humanize_theme_name("tokyo-night_storm"),
            "Tokyo Night Storm"
        );
        assert_eq!(humanize_theme_name("Already Nice"), "Already Nice");
    }
}
