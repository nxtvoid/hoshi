//! What the theme editor opens with and how it saves (`state/theme-editor.ts`).

use super::color::is_color;
use super::model::{
    STANDARD_THEME_ID, THEME_FILE_VERSION, ThemeAppearance, ThemeCollection, ThemeColors,
    ThemeDefinition, ThemeError, ThemeErrorCode, ThemeSelection, ThemeVariants, choose_theme,
    derive_theme_colors, install_theme, parse_theme_file, remove_themes, standard_colors,
    theme_colors_for, theme_id_from_name, theme_modes, update_theme,
};
use serde_json::{Map, Value, json};

/// The two colors the simple mode edits; every other role derives from them.
pub const SIMPLE_ROLES: [&str; 2] = ["background", "primary"];

/// One value for each appearance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PerAppearance<T> {
    pub light: T,
    pub dark: T,
}

impl<T> PerAppearance<T> {
    pub fn get(&self, appearance: ThemeAppearance) -> &T {
        match appearance {
            ThemeAppearance::Light => &self.light,
            ThemeAppearance::Dark => &self.dark,
        }
    }

    pub fn get_mut(&mut self, appearance: ThemeAppearance) -> &mut T {
        match appearance {
            ThemeAppearance::Light => &mut self.light,
            ThemeAppearance::Dark => &mut self.dark,
        }
    }
}

pub type EditorColors = PerAppearance<ThemeColors>;

#[derive(Clone, Debug, PartialEq)]
pub struct EditorStart {
    pub colors: EditorColors,
    pub appearance: ThemeAppearance,
    pub advanced: bool,
    /// Leaving the advanced mode rebuilds the colors from the two simple ones.
    pub regenerate: bool,
}

/// Where the editor opens: the edited or seeded theme's colors, else hoshi's own.
pub fn start_editor(
    source: Option<&ThemeDefinition>,
    initial_appearance: ThemeAppearance,
) -> EditorStart {
    let seed = source.filter(|theme| theme.id != STANDARD_THEME_ID);
    let mut colors = EditorColors {
        light: standard_colors(ThemeAppearance::Light).clone(),
        dark: standard_colors(ThemeAppearance::Dark).clone(),
    };
    if let Some(seed) = seed {
        for appearance in theme_modes(seed) {
            if let Some(seeded) = theme_colors_for(seed, appearance) {
                *colors.get_mut(appearance) = seeded.clone();
            }
        }
    }
    let appearance = match seed {
        Some(seed) if theme_colors_for(seed, initial_appearance).is_none() => seed.appearance,
        _ => initial_appearance,
    };
    let advanced = seed.is_some_and(|seed| !seed.managed);
    EditorStart {
        colors,
        appearance,
        advanced,
        regenerate: advanced,
    }
}

/// The colors the simple mode derives for `appearance` from its background and accent.
pub fn managed_colors(appearance: ThemeAppearance, colors: &ThemeColors) -> ThemeColors {
    let defaults = standard_colors(appearance);
    let simple = |role: &str| {
        let value = colors.get(role);
        if is_color(value.trim()) {
            value
        } else {
            defaults.get(role)
        }
    };
    derive_theme_colors(appearance, simple("background"), simple("primary"))
}

/// A custom theme the name already belongs to, so saving adds a palette to it.
pub fn find_merge_target<'a>(
    name: &str,
    custom_themes: &'a [ThemeDefinition],
    editing_id: Option<&str>,
) -> Option<&'a ThemeDefinition> {
    let normalized = name.trim().to_lowercase();
    if normalized.is_empty() {
        return None;
    }
    let id = theme_id_from_name(name);
    custom_themes.iter().find(|theme| {
        Some(theme.id.as_str()) != editing_id
            && (theme.id == id || theme.label.trim().to_lowercase() == normalized)
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppearanceLockReason {
    /// The edited theme has no colors for that appearance.
    EditingLacks,
    /// The theme the name merges into already has that appearance.
    MergeHas,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppearanceLock {
    pub reason: AppearanceLockReason,
    pub name: String,
    pub appearance: ThemeAppearance,
}

/// Why the editor can't switch to `appearance`, or `None` when it can.
pub fn appearance_lock(
    appearance: ThemeAppearance,
    editing: Option<&ThemeDefinition>,
    merge_target: Option<&ThemeDefinition>,
) -> Option<AppearanceLock> {
    let lock = |reason, theme: &ThemeDefinition| AppearanceLock {
        reason,
        name: theme.label.clone(),
        appearance,
    };
    match (editing, merge_target) {
        (Some(editing), _) if !theme_modes(editing).contains(&appearance) => {
            Some(lock(AppearanceLockReason::EditingLacks, editing))
        }
        (None, Some(target)) if theme_modes(target).contains(&appearance) => {
            Some(lock(AppearanceLockReason::MergeHas, target))
        }
        _ => None,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EditorSave<'a> {
    pub name: &'a str,
    pub editing: Option<&'a ThemeDefinition>,
    pub appearance: ThemeAppearance,
    pub colors: &'a EditorColors,
    pub advanced: bool,
    /// The simple colors changed per appearance, so the derived ones must be rebuilt.
    pub simple_dirty: PerAppearance<bool>,
    pub custom_themes: &'a [ThemeDefinition],
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditorResult {
    pub themes: Vec<ThemeDefinition>,
    pub saved: ThemeDefinition,
    pub created: bool,
    pub merged_appearance: Option<ThemeAppearance>,
    /// What the app shows once saved; `None` leaves the choice as it was.
    pub selection: Option<ThemeSelection>,
}

struct ThemeFile<'a> {
    id: Option<&'a str>,
    name: &'a str,
    appearance: ThemeAppearance,
    colors: &'a ThemeColors,
    variants: ThemeVariants,
    managed: bool,
}

fn theme_file(
    theme: ThemeFile<'_>,
    collection: Option<&ThemeCollection>,
) -> Result<ThemeDefinition, ThemeError> {
    let mut file = Map::new();
    file.insert("version".into(), json!(THEME_FILE_VERSION));
    if let Some(id) = theme.id.filter(|id| !id.is_empty()) {
        file.insert("id".into(), json!(id));
    }
    file.insert("name".into(), json!(theme.name));
    file.insert("appearance".into(), json!(theme.appearance.as_str()));
    file.insert("colors".into(), theme.colors.to_json());
    if !theme.variants.is_empty() {
        let mut variants = Map::new();
        for appearance in ThemeAppearance::ALL {
            if let Some(colors) = theme.variants.get(appearance) {
                variants.insert(appearance.as_str().into(), colors.to_json());
            }
        }
        file.insert("variants".into(), Value::Object(variants));
    }
    if theme.managed {
        file.insert("managed".into(), Value::Bool(true));
    }
    let mut parsed = parse_theme_file(&Value::Object(file))?;
    if let Some(collection) = collection {
        parsed.collection = Some(collection.clone());
    }
    Ok(parsed)
}

/// Saves what the editor holds: a new theme, an edit, or a palette added to a same-named theme.
pub fn save_editor(input: &EditorSave<'_>) -> Result<EditorResult, ThemeError> {
    let EditorSave {
        name,
        editing,
        appearance,
        colors,
        advanced,
        custom_themes,
        ..
    } = *input;
    if name.trim().is_empty() {
        return Err(ThemeError::new(ThemeErrorCode::MissingName));
    }
    let rebuilt = |mode: ThemeAppearance| {
        if !advanced && *input.simple_dirty.get(mode) {
            managed_colors(mode, colors.get(mode))
        } else {
            colors.get(mode).clone()
        }
    };
    let for_save = EditorColors {
        light: rebuilt(ThemeAppearance::Light),
        dark: rebuilt(ThemeAppearance::Dark),
    };
    let merge_target =
        find_merge_target(name, custom_themes, editing.map(|theme| theme.id.as_str()));
    let taken = merge_target.map(theme_modes).unwrap_or_default();
    let merged_variants = |target: &ThemeDefinition, modes: &[ThemeAppearance]| {
        let mut variants = target.variants.clone();
        for mode in modes {
            variants.set(*mode, Some(for_save.get(*mode).clone()));
        }
        variants
    };
    let merge_into = |target: &ThemeDefinition, modes: &[ThemeAppearance]| {
        theme_file(
            ThemeFile {
                id: Some(&target.id),
                name: &target.label,
                appearance: target.appearance,
                colors: &target.colors,
                variants: merged_variants(target, modes),
                managed: target.managed && !advanced,
            },
            target.collection.as_ref(),
        )
    };

    if let (Some(editing), Some(target)) = (editing, merge_target) {
        let edited = theme_modes(editing);
        if let Some(collision) = edited.iter().find(|mode| taken.contains(mode)) {
            return Err(ThemeError {
                appearance: Some(*collision),
                ..ThemeError::about(ThemeErrorCode::PaletteTaken, target.label.clone())
            });
        }
        let saved = merge_into(target, &edited)?;
        return Ok(EditorResult {
            themes: remove_themes(&update_theme(custom_themes, saved.clone())?, &[&editing.id]),
            selection: Some(choose_theme(&saved.id)),
            saved,
            created: false,
            merged_appearance: edited.first().copied(),
        });
    }

    if let Some(editing) = editing {
        let base = editing.appearance;
        let variant = base.other();
        let mut variants = ThemeVariants::default();
        if theme_modes(editing).len() > 1 {
            variants.set(variant, Some(for_save.get(variant).clone()));
        }
        let saved = theme_file(
            ThemeFile {
                id: Some(&editing.id),
                name,
                appearance: base,
                colors: for_save.get(base),
                variants,
                managed: !advanced,
            },
            editing.collection.as_ref(),
        )?;
        return Ok(EditorResult {
            themes: update_theme(custom_themes, saved.clone())?,
            saved,
            created: false,
            merged_appearance: None,
            selection: None,
        });
    }

    if let Some(target) = merge_target {
        if taken.contains(&appearance) {
            return Err(ThemeError::about(
                ThemeErrorCode::PalettesTaken,
                target.label.clone(),
            ));
        }
        let saved = merge_into(target, &[appearance])?;
        return Ok(EditorResult {
            themes: update_theme(custom_themes, saved.clone())?,
            selection: Some(choose_theme(&saved.id)),
            saved,
            created: false,
            merged_appearance: Some(appearance),
        });
    }

    let saved = theme_file(
        ThemeFile {
            id: None,
            name,
            appearance,
            colors: for_save.get(appearance),
            variants: ThemeVariants::default(),
            managed: !advanced,
        },
        None,
    )?;
    Ok(EditorResult {
        themes: install_theme(custom_themes, saved.clone())?,
        selection: Some(choose_theme(&saved.id)),
        saved,
        created: true,
        merged_appearance: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::themes::model::{ThemeHalves, built_in_themes};

    fn dusk() -> ThemeDefinition {
        ThemeDefinition {
            managed: true,
            ..ThemeDefinition::new(
                "dusk",
                "Dusk",
                ThemeAppearance::Dark,
                derive_theme_colors(ThemeAppearance::Dark, "#15131c", "#d08cff"),
            )
        }
    }

    fn grove() -> &'static ThemeDefinition {
        built_in_themes()
            .iter()
            .find(|theme| theme.id == "grove")
            .unwrap()
    }

    fn colors() -> EditorColors {
        EditorColors {
            light: derive_theme_colors(ThemeAppearance::Light, "#fbf8f1", "#b4532a"),
            dark: derive_theme_colors(ThemeAppearance::Dark, "#101018", "#7c6cff"),
        }
    }

    fn save<'a>(
        overrides: impl FnOnce(&mut EditorSave<'a>),
        colors: &'a EditorColors,
        customs: &'a [ThemeDefinition],
    ) -> Result<EditorResult, ThemeError> {
        let mut input = EditorSave {
            name: "Aurora",
            editing: None,
            appearance: ThemeAppearance::Light,
            colors,
            advanced: false,
            simple_dirty: PerAppearance::default(),
            custom_themes: customs,
        };
        overrides(&mut input);
        save_editor(&input)
    }

    #[test]
    fn starts_from_hoshi_in_the_simple_mode() {
        let start = start_editor(None, ThemeAppearance::Dark);
        assert_eq!(start.appearance, ThemeAppearance::Dark);
        assert!(!start.advanced);
        assert_eq!(&start.colors.dark, standard_colors(ThemeAppearance::Dark));
    }

    #[test]
    fn the_standard_theme_as_a_seed_is_the_same_as_none() {
        assert!(!start_editor(Some(&built_in_themes()[0]), ThemeAppearance::Light).advanced);
    }

    #[test]
    fn a_hand_tuned_seed_opens_in_the_advanced_mode() {
        let start = start_editor(Some(grove()), ThemeAppearance::Light);
        assert!(start.advanced);
        assert_eq!(
            start.colors.light.get("primary"),
            grove().colors.get("primary")
        );
    }

    #[test]
    fn a_seed_without_the_wanted_appearance_opens_on_its_own() {
        let start = start_editor(Some(&dusk()), ThemeAppearance::Light);
        assert_eq!(start.appearance, ThemeAppearance::Dark);
        assert!(!start.advanced);
    }

    #[test]
    fn find_merge_target_matches_by_name_or_id_but_not_the_one_being_edited() {
        let customs = [dusk()];
        assert_eq!(
            find_merge_target(" dusk ", &customs, None),
            Some(&customs[0])
        );
        assert_eq!(find_merge_target("Dusk", &customs, Some("dusk")), None);
        assert_eq!(find_merge_target("", &customs, None), None);
    }

    #[test]
    fn an_edited_theme_keeps_to_its_own_appearances() {
        let dusk = dusk();
        assert_eq!(
            appearance_lock(ThemeAppearance::Light, Some(&dusk), None).map(|lock| lock.reason),
            Some(AppearanceLockReason::EditingLacks)
        );
        assert_eq!(
            appearance_lock(ThemeAppearance::Dark, Some(&dusk), None),
            None
        );
    }

    #[test]
    fn a_new_theme_cannot_take_a_palette_its_namesake_has() {
        let dusk = dusk();
        assert_eq!(
            appearance_lock(ThemeAppearance::Dark, None, Some(&dusk)).map(|lock| lock.reason),
            Some(AppearanceLockReason::MergeHas)
        );
        assert_eq!(
            appearance_lock(ThemeAppearance::Light, None, Some(&dusk)),
            None
        );
    }

    #[test]
    fn creates_a_single_appearance_theme_and_makes_it_active() {
        let result = save(|_| {}, &colors(), &[dusk()]).unwrap();
        assert!(result.created);
        assert_eq!(result.saved.id, "aurora");
        assert_eq!(theme_modes(&result.saved), [ThemeAppearance::Light]);
        assert!(result.saved.managed);
        assert_eq!(
            result.selection,
            Some(ThemeSelection {
                theme: "aurora".into(),
                halves: ThemeHalves::default()
            })
        );
        let ids: Vec<&str> = result
            .themes
            .iter()
            .map(|theme| theme.id.as_str())
            .collect();
        assert_eq!(ids, ["dusk", "aurora"]);
    }

    #[test]
    fn rebuilds_the_simple_colors_it_changed() {
        let mut edited = colors();
        edited.light.set("background", "#ffffff");
        edited.light.set("primary", "#ff0000");
        let result = save(
            |input| {
                input.simple_dirty = PerAppearance {
                    light: true,
                    dark: false,
                }
            },
            &edited,
            &[dusk()],
        )
        .unwrap();
        assert_eq!(
            result.saved.colors,
            derive_theme_colors(ThemeAppearance::Light, "#ffffff", "#ff0000")
        );
    }

    #[test]
    fn a_name_it_already_has_adds_the_missing_palette() {
        let result = save(|input| input.name = "Dusk", &colors(), &[dusk()]).unwrap();
        assert_eq!(result.merged_appearance, Some(ThemeAppearance::Light));
        assert_eq!(
            theme_modes(&result.saved),
            [ThemeAppearance::Light, ThemeAppearance::Dark]
        );
        assert_eq!(result.themes.len(), 1);
    }

    #[test]
    fn refuses_a_palette_its_namesake_already_has() {
        let error = save(
            |input| {
                input.name = "Dusk";
                input.appearance = ThemeAppearance::Dark;
            },
            &colors(),
            &[dusk()],
        )
        .unwrap_err();
        assert_eq!(error.code, ThemeErrorCode::PalettesTaken);
    }

    #[test]
    fn an_edit_keeps_its_id_and_its_choice() {
        let dusk = dusk();
        let result = save(
            |input| {
                input.name = "Dusk at sea";
                input.editing = Some(&dusk);
                input.appearance = ThemeAppearance::Dark;
                input.advanced = true;
            },
            &colors(),
            std::slice::from_ref(&dusk),
        )
        .unwrap();
        assert_eq!(result.saved.id, "dusk");
        assert_eq!(result.saved.label, "Dusk at sea");
        assert!(!result.saved.managed);
        assert_eq!(result.selection, None);
    }

    #[test]
    fn renaming_an_edit_onto_a_namesake_merges_into_it() {
        let dawn = ThemeDefinition::new("dawn", "Dawn", ThemeAppearance::Light, colors().light);
        let customs = [dusk(), dawn.clone()];
        let result = save(
            |input| {
                input.name = "Dusk";
                input.editing = Some(&dawn);
            },
            &colors(),
            &customs,
        )
        .unwrap();
        assert_eq!(result.merged_appearance, Some(ThemeAppearance::Light));
        let ids: Vec<&str> = result
            .themes
            .iter()
            .map(|theme| theme.id.as_str())
            .collect();
        assert_eq!(ids, ["dusk"]);
        assert_eq!(
            theme_modes(&result.themes[0]),
            [ThemeAppearance::Light, ThemeAppearance::Dark]
        );
    }

    #[test]
    fn needs_a_name() {
        let error = save(|input| input.name = "  ", &colors(), &[dusk()]).unwrap_err();
        assert_eq!(error.code, ThemeErrorCode::MissingName);
    }
}
