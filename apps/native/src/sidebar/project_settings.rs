//! A project's own settings, over the app's: whether its threads continue after a restart, and
//! its worktree setup.

use super::Sidebar;
use crate::m;
use gpui_kit::*;
use hoshi_core::{Command, EffectiveSetting, SettingSource};
use hoshi_ui::prelude::*;

/// What the select shows: "Inherit (On)" while the project keeps the app's value.
fn choice_label(choice: Option<bool>, global: bool) -> String {
    match choice {
        None if global => m::setting_inherit_on(),
        None => m::setting_inherit_off(),
        Some(true) => m::setting_on(),
        Some(false) => m::setting_off(),
    }
}

/// "Set for this project" while the project overrides the app's value; the select already
/// says what it inherits.
fn source_label(effective: EffectiveSetting<bool>) -> Option<String> {
    (effective.source == SettingSource::Project).then(m::setting_set_for_project)
}

impl Sidebar {
    pub(super) fn open_project_settings(&mut self, project_id: String, cx: &mut Context<Self>) {
        self.project_settings = Some(project_id);
        cx.notify();
    }

    fn set_continue_after_restart(
        &mut self,
        project_id: &str,
        choice: Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.workspace.read(cx).model.project(project_id) else {
            return;
        };
        let mut settings = project.settings.clone();
        settings.continue_after_restart = choice;
        self.dispatch(
            Command::ProjectSettingsSet {
                project_id: project_id.to_owned(),
                settings,
            },
            cx,
        );
    }

    pub(super) fn project_settings_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let project_id = self.project_settings.clone()?;
        let workspace = self.workspace.read(cx);
        let project = workspace.model.project(&project_id)?.clone();
        let global = workspace.engine.continue_after_restart_in(None).value;
        let effective = workspace
            .engine
            .continue_after_restart_in(Some(&project_id));
        let chosen = project.settings.continue_after_restart;
        let this = cx.weak_entity();
        let options = [None, Some(true), Some(false)].map(|choice| {
            let (this, project_id) = (this.clone(), project_id.clone());
            let key = match choice {
                None => "project-continue-inherit",
                Some(true) => "project-continue-on",
                Some(false) => "project-continue-off",
            };
            SelectOption::new(key, choice_label(choice, global), chosen == choice).on_select(
                move |_, cx| {
                    this.update(cx, |this, cx| {
                        this.set_continue_after_restart(&project_id, choice, cx)
                    })
                    .ok();
                },
            )
        });
        let select = Select::new(
            "project-continue-after-restart",
            choice_label(chosen, global),
        )
        .align(Align::End)
        .min_width(px(128.))
        .options(options);
        let description = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(m::project_continue_after_restart_description())
            .children(source_label(effective).map(|source| {
                div()
                    .debug_selector(|| "project-continue-source".into())
                    .child(source)
            }));
        let edit = div()
            .debug_selector(|| "project-worktree-setup-edit".into())
            .child(
                Button::new("project-worktree-setup-edit")
                    .label(m::project_worktree_setup_edit())
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Outline)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.project_settings = None;
                        this.edit_setup(&project_id, window, cx);
                    })),
            );
        Some(
            Dialog::new("project-settings")
                .open(true)
                .size(DialogSize::Custom(px(512.)))
                .on_open_change(cx.listener(|this, change: &OpenChange, _, cx| {
                    if !change.open {
                        this.project_settings = None;
                        cx.notify();
                    }
                }))
                .title(m::project_settings_title(project.title))
                .description(m::project_settings_description())
                .child(
                    SettingsGroup::new()
                        .child(
                            SettingsRow::new(m::project_continue_after_restart())
                                .description(description)
                                .control(select),
                        )
                        .child(
                            SettingsRow::new(m::project_worktree_setup())
                                .description(m::project_worktree_setup_description())
                                .control(edit),
                        ),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{choice_label, source_label};
    use hoshi_core::{EffectiveSetting, SettingSource};

    #[test]
    fn inheriting_shows_the_value_it_inherits() {
        assert_eq!(choice_label(None, true), "Inherit (On)");
        assert_eq!(choice_label(None, false), "Inherit (Off)");
        assert_eq!(choice_label(Some(false), true), "Off");
        assert_eq!(
            source_label(EffectiveSetting {
                value: true,
                source: SettingSource::Global
            }),
            None
        );
        assert_eq!(
            source_label(EffectiveSetting {
                value: false,
                source: SettingSource::Project
            })
            .as_deref(),
            Some("Set for this project")
        );
    }
}
