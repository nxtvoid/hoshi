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
