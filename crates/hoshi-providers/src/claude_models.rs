//! Claude's model manifest: which models exist, what each one supports, and
//! which Claude Code release each needs.

use hoshi_core::{Effort, ModelInfo, ProviderModels};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

const MANIFEST: &str = include_str!("claude-models.json");
const NEW_FOR_DAYS: i64 = 7;
pub(crate) const PROVIDER: &str = "claudeCode";

pub(crate) type Version = (u32, u32, u32);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Manifest {
    defaults: Defaults,
    models: Vec<ManifestModel>,
    profiles: HashMap<String, Profile>,
}

#[derive(Deserialize)]
struct Defaults {
    chat: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestModel {
    slug: String,
    name: String,
    short_name: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
    status: Status,
    released_at: Option<String>,
    profile: String,
    #[serde(default)]
    adapter: ModelAdapter,
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum Status {
    Current,
    Legacy,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ModelAdapter {
    claude_code: Option<VersionRange>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionRange {
    min_version: Option<String>,
    max_version_exclusive: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    capabilities: Capabilities,
    #[serde(default)]
    adapter: ProfileAdapter,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Capabilities {
    option_descriptors: Vec<Descriptor>,
}

#[derive(Deserialize)]
struct Descriptor {
    id: String,
    #[serde(default)]
    options: Vec<DescriptorOption>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DescriptorOption {
    id: String,
    #[serde(default)]
    is_default: bool,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ProfileAdapter {
    claude_code: Option<ClaudeProfile>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ClaudeProfile {
    /// Effort values Claude Code knows under another name; `null` sends none.
    #[serde(default)]
    effort_map: HashMap<String, Option<String>>,
    /// Set for models with one window size; others are known once a turn reports it.
    fixed_context_window_tokens: Option<u64>,
}

/// What a model's profile offers.
struct Traits {
    efforts: Vec<Effort>,
    default_effort: Option<Effort>,
    fast_mode: bool,
    ultracode: bool,
}

/// The model and options a turn runs with, in Claude Code's terms.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TurnModel {
    pub slug: String,
    pub effort: Option<String>,
    pub fast_mode: bool,
    pub ultracode: bool,
}

pub(crate) fn manifest() -> &'static Manifest {
    static PARSED: OnceLock<Manifest> = OnceLock::new();
    PARSED.get_or_init(|| Manifest::parse(MANIFEST).expect("the bundled manifest is valid"))
}

impl Manifest {
    fn parse(text: &str) -> serde_json::Result<Self> {
        serde_json::from_str(text)
    }

    fn find(&self, id: &str) -> Option<&ManifestModel> {
        self.models
            .iter()
            .find(|model| model.slug == id)
            .or_else(|| {
                self.models.iter().find(|model| {
                    model
                        .aliases
                        .iter()
                        .any(|alias| alias.eq_ignore_ascii_case(id))
                })
            })
    }

    fn traits(&self, model: &ManifestModel) -> Traits {
        let descriptors = self
            .profiles
            .get(&model.profile)
            .map(|profile| profile.capabilities.option_descriptors.as_slice())
            .unwrap_or_default();
        let effort = descriptors
            .iter()
            .find(|descriptor| descriptor.id == "effort");
        let options = effort
            .map(|effort| effort.options.as_slice())
            .unwrap_or_default();
        let ultracode = options.iter().any(|option| option.id == "ultracode");
        let default_effort = options
            .iter()
            .find(|option| option.is_default)
            .and_then(|option| match option.id.as_str() {
                "ultracode" => Some(Effort::Xhigh),
                id => parse_effort(id),
            });
        Traits {
            efforts: options
                .iter()
                .filter_map(|option| parse_effort(&option.id))
                .collect(),
            default_effort,
            fast_mode: descriptors
                .iter()
                .any(|descriptor| descriptor.id == "fastMode"),
            ultracode,
        }
    }

    fn effort_map(&self, model: &ManifestModel) -> Option<&HashMap<String, Option<String>>> {
        self.profiles
            .get(&model.profile)?
            .adapter
            .claude_code
            .as_ref()
            .map(|claude| &claude.effort_map)
    }

    /// The context window of `slug`, when the manifest knows it before a turn reports it.
    pub(crate) fn context_window(&self, slug: &str) -> Option<u64> {
        let model = self.find(slug)?;
        self.profiles
            .get(&model.profile)?
            .adapter
            .claude_code
            .as_ref()?
            .fixed_context_window_tokens
    }

    /// Fills in the defaults and drops what the model doesn't support. A model
    /// the manifest doesn't know passes through unchanged.
    pub(crate) fn resolve(
        &self,
        model: Option<&str>,
        effort: Option<Effort>,
        fast_mode: bool,
        ultracode: bool,
    ) -> TurnModel {
        let id = model.unwrap_or(&self.defaults.chat);
        let Some(entry) = self.find(id) else {
            return TurnModel {
                slug: id.to_owned(),
                effort: effort.map(|effort| effort.as_str().to_owned()),
                fast_mode,
                ultracode,
            };
        };
        let traits = self.traits(entry);
        let effort = effort
            .filter(|effort| traits.efforts.contains(effort))
            .or(traits.default_effort)
            .and_then(|effort| {
                match self
                    .effort_map(entry)
                    .and_then(|map| map.get(effort.as_str()))
                {
                    Some(mapped) => mapped.clone(),
                    None => Some(effort.as_str().to_owned()),
                }
            });
        TurnModel {
            slug: entry.slug.clone(),
            effort,
            fast_mode: fast_mode && traits.fast_mode,
            ultracode: ultracode && traits.ultracode,
        }
    }

    /// `version` is the installed Claude Code; when unknown, every model is offered.
    /// `today` counts days since the Unix epoch.
    pub(crate) fn catalog(&self, version: Option<Version>, today: i64) -> ProviderModels {
        let models = self
            .models
            .iter()
            .filter_map(|model| {
                let range = model.adapter.claude_code.as_ref();
                let too_old = |version: Version| {
                    range
                        .and_then(|range| range.min_version.as_deref())
                        .and_then(parse_version)
                        .is_some_and(|min| version < min)
                };
                let too_new = |version: Version| {
                    range
                        .and_then(|range| range.max_version_exclusive.as_deref())
                        .and_then(parse_version)
                        .is_some_and(|max| version >= max)
                };
                let legacy = model.status == Status::Legacy;
                let update_required = version.is_some_and(too_old);
                if (update_required && legacy) || version.is_some_and(too_new) {
                    return None;
                }
                let traits = self.traits(model);
                Some(ModelInfo {
                    slug: model.slug.clone(),
                    name: model.name.clone(),
                    short_name: model.short_name.clone().unwrap_or_else(|| {
                        model
                            .name
                            .strip_prefix("Claude ")
                            .unwrap_or(&model.name)
                            .to_owned()
                    }),
                    legacy,
                    is_new: !legacy
                        && model
                            .released_at
                            .as_deref()
                            .and_then(days_since_epoch)
                            .is_some_and(|released| {
                                (0..NEW_FOR_DAYS).contains(&(today - released))
                            }),
                    update_required,
                    efforts: traits.efforts,
                    default_effort: traits.default_effort,
                    fast_mode: traits.fast_mode,
                    ultracode: traits.ultracode,
                })
            })
            .collect();
        ProviderModels {
            provider: PROVIDER.to_owned(),
            default_model: self
                .find(&self.defaults.chat)
                .map(|model| model.slug.clone()),
            models,
        }
    }
}

fn parse_effort(id: &str) -> Option<Effort> {
    serde_json::from_value(id.into()).ok()
}

/// The first `x.y.z` in `text`, as `claude --version` prints it.
pub(crate) fn parse_version(text: &str) -> Option<Version> {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .find_map(|word| {
            let mut parts = word.split('.').map(str::parse::<u32>);
            match (parts.next(), parts.next(), parts.next(), parts.next()) {
                (Some(Ok(major)), Some(Ok(minor)), Some(Ok(patch)), None) => {
                    Some((major, minor, patch))
                }
                _ => None,
            }
        })
}

/// Days since the Unix epoch for a `YYYY-MM-DD` date.
pub(crate) fn days_since_epoch(date: &str) -> Option<i64> {
    let mut parts = date.get(..10)?.split('-').map(str::parse::<i64>);
    let (Some(Ok(year)), Some(Ok(month)), Some(Ok(day))) =
        (parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: Version = (2, 1, 318);

    #[test]
    fn the_bundled_manifest_parses() {
        assert!(Manifest::parse(MANIFEST).is_ok());
    }

    #[test]
    fn aliases_resolve_to_full_slugs() {
        let turn = manifest().resolve(Some("opus"), None, false, false);
        assert_eq!(turn.slug, "claude-opus-5-5");
        assert_eq!(turn.effort.as_deref(), Some("medium"));
        assert_eq!(
            manifest().resolve(None, None, false, false).slug,
            "claude-opus-5-5"
        );
    }

    #[test]
    fn only_models_with_one_window_size_know_it_up_front() {
        assert_eq!(manifest().context_window("opus"), Some(1_000_000));
        assert_eq!(manifest().context_window("claude-opus-4-5"), Some(200_000));
        assert_eq!(manifest().context_window("claude-opus-4-6"), None);
        assert_eq!(manifest().context_window("unknown-model"), None);
    }

    #[test]
    fn unsupported_options_are_dropped() {
        let turn = manifest().resolve(Some("claude-sonnet-5-5"), None, true, true);
        assert!(!turn.fast_mode);
        assert!(turn.ultracode);
        let turn = manifest().resolve(Some("claude-opus-4-6"), Some(Effort::Xhigh), true, true);
        assert_eq!(turn.effort.as_deref(), Some("high"));
        assert!(turn.fast_mode && !turn.ultracode);
    }

    #[test]
    fn efforts_follow_the_profile_map() {
        let turn = manifest().resolve(Some("claude-opus-4-7"), Some(Effort::Xhigh), false, false);
        assert_eq!(turn.effort.as_deref(), Some("max"));
    }

    #[test]
    fn fable_means_fable_5_2_which_has_fast_mode_and_needs_a_newer_cli() {
        assert_eq!(
            manifest().resolve(Some("fable"), None, false, false).slug,
            "claude-fable-5-2"
        );
        let today = days_since_epoch("2026-11-09").unwrap();
        let find = |catalog: &ProviderModels, slug: &str| {
            catalog
                .models
                .iter()
                .find(|model| model.slug == slug)
                .cloned()
        };
        let current = manifest().catalog(Some((2, 1, 321)), today);
        let fable = find(&current, "claude-fable-5-2").unwrap();
        assert!(fable.fast_mode && !fable.update_required);
        let older = manifest().catalog(Some((2, 1, 319)), today);
        assert!(find(&older, "claude-fable-5-2").unwrap().update_required);
    }

    #[test]
    fn unknown_models_pass_through() {
        let turn = manifest().resolve(Some("claude-custom"), Some(Effort::Low), true, false);
        assert_eq!(turn.slug, "claude-custom");
        assert_eq!(turn.effort.as_deref(), Some("low"));
        assert!(turn.fast_mode);
    }

    #[test]
    fn the_catalog_follows_the_installed_version() {
        let today = days_since_epoch("2026-11-07").unwrap();
        let all = manifest().catalog(Some(V), today);
        let opus = all
            .models
            .iter()
            .find(|model| model.slug == "claude-opus-5-5")
            .unwrap();
        assert_eq!(opus.short_name, "Opus 5.5");
        assert!(opus.fast_mode && opus.ultracode && !opus.update_required);
        assert_eq!(all.default_model.as_deref(), Some("claude-opus-5-5"));

        let old = manifest().catalog(Some((2, 1, 200)), today);
        let sonnet = old
            .models
            .iter()
            .find(|model| model.slug == "claude-sonnet-5-5")
            .unwrap();
        assert!(sonnet.update_required);
        assert!(!old.models.iter().any(|model| model.slug == "claude-opus-5"));
    }

    #[test]
    fn a_model_is_new_for_a_week() {
        let released = days_since_epoch("2026-10-07").unwrap();
        let haiku = |today| {
            manifest()
                .catalog(Some(V), today)
                .models
                .into_iter()
                .find(|model| model.slug == "claude-haiku-5-5")
                .unwrap()
                .is_new
        };
        assert!(haiku(released + 6));
        assert!(!haiku(released + 7));
    }

    #[test]
    fn reads_versions_and_dates() {
        assert_eq!(parse_version("2.1.318 (Claude Code)"), Some(V));
        assert_eq!(parse_version("claude v10.0.1"), Some((10, 0, 1)));
        assert_eq!(parse_version("no version"), None);
        assert_eq!(days_since_epoch("1970-01-02"), Some(1));
        assert_eq!(days_since_epoch("2026-11-07"), Some(20_764));
    }
}
