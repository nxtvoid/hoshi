//! The thread menu's "Continue with…": the other providers' models, which the thread moves to
//! on its next message, taking its conversation with it.

use crate::composer::{command_error_message, fit_choice};
use crate::m;
use crate::request_rows::provider_name;
use crate::state::{ModelChoice, Workspace};
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::{Command, ModelInfo, ProviderModels};
use hoshi_ui::prelude::*;

/// The models of the enabled providers other than `current`, by provider; legacy models and
/// providers left with none are left out.
pub fn other_models<'a>(
    catalogs: &'a [ProviderModels],
    current: &str,
) -> Vec<(&'a str, Vec<&'a ModelInfo>)> {
    catalogs
        .iter()
        .filter(|catalog| catalog.provider != current)
        .map(|catalog| {
            let models = catalog
                .models
                .iter()
                .filter(|model| !model.legacy)
                .collect::<Vec<_>>();
            (catalog.provider.as_str(), models)
        })
        .filter(|(_, models)| !models.is_empty())
        .collect()
}

/// What a thread on `current` continues with on `model`: its effort and speed, as far as the
/// model takes them.
fn choice_for(current: &hoshi_core::ModelChoice, provider: &str, model: &ModelInfo) -> ModelChoice {
    fit_choice(
        ModelChoice {
            provider: provider.to_owned(),
            model: Some(model.slug.clone()),
            effort: current.effort,
            fast_mode: current.fast_mode,
            ultracode: current.ultracode,
        },
        Some(model),
    )
}

/// The submenu's models, under their provider's name.
pub fn continue_with_menu(
    mut menu: PopoverMenu,
    workspace: Entity<Workspace>,
    thread_id: &str,
    cx: &App,
) -> PopoverMenu {
    let state = workspace.read(cx);
    let Some(thread) = state.model.thread(thread_id) else {
        return menu;
    };
    let pending = thread.pending_model.as_deref();
    let current = hoshi_core::ModelChoice::of(thread);
    for (provider, models) in other_models(&state.models, &thread.provider) {
        menu = menu.heading(provider_name(provider));
        for model in models {
            let choice = choice_for(&current, provider, model);
            let picked = pending.is_some_and(|pending| {
                pending.provider == provider && pending.model.as_deref() == Some(&model.slug)
            });
            let (workspace, thread_id) = (workspace.clone(), thread.id.clone());
            let option = MenuOption::new(
                SharedString::from(format!("continue-with-{provider}-{}", model.slug)),
                model.name.clone(),
            )
            .disabled(model.update_required)
            .on_select(move |window, cx| {
                continue_with(
                    workspace.clone(),
                    thread_id.clone(),
                    choice.clone(),
                    window,
                    cx,
                )
            });
            menu = menu.item(if picked {
                option.trailing(Icon::new(IconName::Check).size_3p5())
            } else {
                option
            });
        }
    }
    menu
}

/// Has the thread move to `choice`: at once while it has no conversation, else with its next
/// message.
pub fn continue_with(
    workspace: Entity<Workspace>,
    thread_id: String,
    choice: ModelChoice,
    window: &mut Window,
    cx: &mut App,
) {
    let engine = workspace.read(cx).engine.clone();
    let command = Command::ThreadModelSet {
        thread_id,
        provider: choice.provider,
        model: choice.model,
        effort: choice.effort,
        fast_mode: choice.fast_mode,
        ultracode: choice.ultracode,
    };
    window
        .spawn(cx, async move |cx| {
            let result = cx
                .background_spawn(async move { engine.dispatch(command) })
                .await;
            let Err(error) = result else {
                return;
            };
            cx.update(|window, cx| {
                let notice = ConfirmRequest::new(m::continue_with_failed(), m::dialog_ok())
                    .description(command_error_message(&error))
                    .tone(ConfirmTone::Warning);
                DialogLayer::confirm(notice, window, cx).detach();
            })
            .ok();
        })
        .detach();
}

#[cfg(test)]
mod tests {
    use super::{choice_for, other_models};
    use crate::state::ModelChoice;
    use hoshi_core::{Effort, ModelInfo, ProviderModels};

    fn model(slug: &str, legacy: bool) -> ModelInfo {
        ModelInfo {
            slug: slug.into(),
            name: slug.into(),
            short_name: slug.into(),
            legacy,
            is_new: false,
            update_required: false,
            efforts: vec![Effort::Low, Effort::High],
            default_effort: None,
            fast_mode: false,
            ultracode: false,
        }
    }

    fn catalog(provider: &str, models: Vec<ModelInfo>) -> ProviderModels {
        ProviderModels {
            provider: provider.into(),
            models,
            ..Default::default()
        }
    }

    #[test]
    fn only_the_other_providers_current_models_are_offered() {
        let catalogs = [
            catalog("claudeCode", vec![model("opus", false)]),
            catalog("codex", vec![model("gpt", false), model("old", true)]),
            catalog("copilot", vec![model("older", true)]),
        ];
        let offered: Vec<(&str, Vec<&str>)> = other_models(&catalogs, "claudeCode")
            .into_iter()
            .map(|(provider, models)| {
                let slugs = models.iter().map(|model| model.slug.as_str()).collect();
                (provider, slugs)
            })
            .collect();
        assert_eq!(offered, [("codex", vec!["gpt"])]);
    }

    #[test]
    fn the_switch_keeps_the_effort_the_new_model_takes() {
        let current = hoshi_core::ModelChoice {
            provider: "claudeCode".into(),
            model: None,
            effort: Some(Effort::High),
            fast_mode: true,
            ultracode: false,
        };
        assert_eq!(
            choice_for(&current, "codex", &model("gpt", false)),
            ModelChoice {
                provider: "codex".into(),
                model: Some("gpt".into()),
                effort: Some(Effort::High),
                fast_mode: false,
                ultracode: false,
            }
        );
    }
}
