//! What a new thread starts with, kept across restarts in `composer.json` beside the settings,
//! with the shape of the web's `hoshi-composer` storage.

use crate::state::{DraftChoices, ModelChoice};
use hoshi_core::{AccessMode, Effort, NewThreadSettings, NewThreadWorkspace};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Workspace {
    #[default]
    Local,
    NewWorktree,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Stored {
    provider: Option<String>,
    model: Option<String>,
    effort: Option<Effort>,
    fast_mode: bool,
    ultracode: bool,
    access_mode: AccessMode,
    plan_mode: bool,
    workspace: Workspace,
}

/// Where a new thread is started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewThreadPlace {
    Chat,
    /// A project with no thread yet, such as a repository just created, which may have no
    /// commit for a worktree to start from.
    EmptyProject,
    Project,
}

/// What a new thread starts with: what the settings fix, else what the last one was started
/// with. Chats and projects without threads run in their folder.
pub fn for_new_thread(
    last_used: &DraftChoices,
    settings: &NewThreadSettings,
    place: NewThreadPlace,
) -> DraftChoices {
    let model = settings.model.as_ref().map_or_else(
        || last_used.model.clone(),
        |fixed| ModelChoice {
            provider: fixed.provider.clone(),
            model: Some(fixed.model.clone()),
            effort: fixed.effort,
            fast_mode: fixed.fast_mode,
            ultracode: fixed.ultracode,
        },
    );
    let worktree = settings.workspace.map_or(last_used.worktree, |workspace| {
        workspace == NewThreadWorkspace::NewWorktree
    });
    DraftChoices {
        model,
        access_mode: settings.access_mode.unwrap_or(last_used.access_mode),
        plan_mode: last_used.plan_mode,
        worktree: worktree && place == NewThreadPlace::Project,
    }
}

pub fn file(home: &Path) -> PathBuf {
    home.join("composer.json")
}

pub fn parse(text: &str) -> DraftChoices {
    let stored: Stored = serde_json::from_str(text).unwrap_or_default();
    DraftChoices {
        model: ModelChoice {
            provider: stored.provider.unwrap_or_else(hoshi_core::default_provider),
            model: stored.model,
            effort: stored.effort,
            fast_mode: stored.fast_mode,
            ultracode: stored.ultracode,
        },
        access_mode: stored.access_mode,
        plan_mode: stored.plan_mode,
        worktree: stored.workspace == Workspace::NewWorktree,
    }
}

pub fn to_json(choices: &DraftChoices) -> String {
    let stored = Stored {
        provider: Some(choices.model.provider.clone()),
        model: choices.model.model.clone(),
        effort: choices.model.effort,
        fast_mode: choices.model.fast_mode,
        ultracode: choices.model.ultracode,
        access_mode: choices.access_mode,
        plan_mode: choices.plan_mode,
        workspace: if choices.worktree {
            Workspace::NewWorktree
        } else {
            Workspace::Local
        },
    };
    serde_json::to_string(&stored).unwrap_or_default()
}

pub fn load(home: &Path) -> DraftChoices {
    std::fs::read_to_string(file(home))
        .map(|text| parse(&text))
        .unwrap_or_default()
}

pub fn save(home: &Path, choices: &DraftChoices) -> std::io::Result<()> {
    std::fs::write(file(home), to_json(choices))
}

#[cfg(test)]
mod tests {
    use super::{NewThreadPlace, for_new_thread, parse, to_json};
    use crate::state::{DraftChoices, ModelChoice};
    use hoshi_core::{AccessMode, Effort, NewThreadModel, NewThreadSettings, NewThreadWorkspace};

    fn last_used() -> DraftChoices {
        DraftChoices {
            model: ModelChoice {
                provider: "copilot".into(),
                model: Some("sonnet".into()),
                effort: Some(Effort::Low),
                fast_mode: true,
                ultracode: false,
            },
            access_mode: AccessMode::Ask,
            plan_mode: true,
            worktree: true,
        }
    }

    #[test]
    fn unset_defaults_keep_what_the_last_thread_used() {
        let choices = for_new_thread(
            &last_used(),
            &NewThreadSettings::default(),
            NewThreadPlace::Project,
        );
        assert_eq!(choices, last_used());
    }

    #[test]
    fn fixed_defaults_override_what_the_last_thread_used() {
        let settings = NewThreadSettings {
            model: Some(NewThreadModel {
                provider: "claudeCode".into(),
                model: "opus".into(),
                effort: Some(Effort::High),
                fast_mode: false,
                ultracode: true,
            }),
            access_mode: Some(AccessMode::FullAccess),
            workspace: Some(NewThreadWorkspace::Local),
        };
        let choices = for_new_thread(&last_used(), &settings, NewThreadPlace::Project);
        assert_eq!(
            choices.model,
            ModelChoice {
                provider: "claudeCode".into(),
                model: Some("opus".into()),
                effort: Some(Effort::High),
                fast_mode: false,
                ultracode: true,
            }
        );
        assert_eq!(choices.access_mode, AccessMode::FullAccess);
        assert!(choices.plan_mode);
        assert!(!choices.worktree);
    }

    #[test]
    fn chats_and_projects_without_threads_start_local() {
        let settings = NewThreadSettings {
            workspace: Some(NewThreadWorkspace::NewWorktree),
            ..NewThreadSettings::default()
        };
        let mut local = last_used();
        local.worktree = false;
        for place in [NewThreadPlace::Chat, NewThreadPlace::EmptyProject] {
            assert!(!for_new_thread(&last_used(), &settings, place).worktree);
            assert!(!for_new_thread(&last_used(), &NewThreadSettings::default(), place).worktree);
        }
        assert!(for_new_thread(&local, &settings, NewThreadPlace::Project).worktree);
    }

    #[test]
    fn choices_read_back_what_was_written() {
        let choices = DraftChoices {
            model: ModelChoice {
                provider: "copilot".into(),
                model: Some("opus".into()),
                effort: Some(Effort::High),
                fast_mode: true,
                ultracode: false,
            },
            access_mode: AccessMode::FullAccess,
            plan_mode: true,
            worktree: true,
        };
        assert_eq!(parse(&to_json(&choices)), choices);
    }

    #[test]
    fn the_web_shape_reads_and_gaps_fall_back_to_defaults() {
        let choices = parse(r#"{"model":"sonnet","accessMode":"auto","workspace":"newWorktree"}"#);
        assert_eq!(choices.model.model.as_deref(), Some("sonnet"));
        assert_eq!(choices.access_mode, AccessMode::Auto);
        assert!(choices.worktree);
        assert!(!choices.plan_mode);
        assert_eq!(parse("not json"), DraftChoices::default());
    }
}
