use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ts_rs::TS;

/// A font family installed on the machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SystemFont {
    pub family: String,
    /// Every face of the family is fixed-width.
    pub monospace: bool,
}

/// One family per name, sorted ignoring case. Takes each face's family and whether the face
/// is fixed-width. Names starting with a dot are macOS UI faces that web content can't use.
pub fn group_font_families<I, S>(faces: I) -> Vec<SystemFont>
where
    I: IntoIterator<Item = (S, bool)>,
    S: AsRef<str>,
{
    let mut families: HashMap<String, SystemFont> = HashMap::new();
    for (family, monospace) in faces {
        let family = family.as_ref().trim();
        if family.is_empty() || family.starts_with('.') {
            continue;
        }
        families
            .entry(family.to_lowercase())
            .and_modify(|font| font.monospace &= monospace)
            .or_insert_with(|| SystemFont {
                family: family.to_owned(),
                monospace,
            });
    }
    let mut fonts: Vec<SystemFont> = families.into_values().collect();
    fonts.sort_by(|left, right| {
        left.family
            .to_lowercase()
            .cmp(&right.family.to_lowercase())
            .then_with(|| left.family.cmp(&right.family))
    });
    fonts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn font(family: &str, monospace: bool) -> SystemFont {
        SystemFont {
            family: family.to_owned(),
            monospace,
        }
    }

    #[test]
    fn faces_of_one_family_collapse_into_one_entry() {
        let fonts = group_font_families([
            ("Cascadia Code", true),
            ("Cascadia Code", true),
            ("Arial", false),
        ]);

        assert_eq!(
            fonts,
            vec![font("Arial", false), font("Cascadia Code", true)]
        );
    }

    #[test]
    fn a_family_is_monospace_only_when_every_face_is() {
        let fonts = group_font_families([("Mixed", true), ("Mixed", false), ("Mono", true)]);

        assert_eq!(fonts, vec![font("Mixed", false), font("Mono", true)]);
    }

    #[test]
    fn families_sort_ignoring_case() {
        let fonts =
            group_font_families([("consolas", true), ("Arial", false), ("Bahnschrift", false)]);

        let names: Vec<&str> = fonts.iter().map(|font| font.family.as_str()).collect();
        assert_eq!(names, ["Arial", "Bahnschrift", "consolas"]);
    }

    #[test]
    fn names_differing_only_in_case_are_one_family() {
        let fonts = group_font_families([("Fira Code", true), ("FIRA CODE", true)]);

        assert_eq!(fonts, vec![font("Fira Code", true)]);
    }

    #[test]
    fn hidden_and_blank_families_are_skipped() {
        let fonts = group_font_families([(".SF NS", false), ("  ", false), (" Menlo ", true)]);

        assert_eq!(fonts, vec![font("Menlo", true)]);
    }
}
