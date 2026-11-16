use super::{SettingsView, field, text_area};
use crate::composer::{
    ACCESS_MODES, ModelTab, PickerHost, PickerList, access_description, access_label, default_tab,
    effective_effort, effort_label, model_options, provider_mark, provider_tint, workspace_option,
};
use crate::folder_picker;
use crate::m;
use crate::state::{ModelChoice, Workspace};
use chrono::{DateTime, Utc};
use gpui_kit::component::Icon;
use gpui_kit::component::input::InputState;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_core::{Effort, NewThreadModel, NewThreadWorkspace};
use hoshi_engine::{KeptReason, MergedCleanup};
use hoshi_ui::prelude::*;

const INACTIVE_DAYS: [u32; 5] = [1, 3, 7, 14, 30];

fn days_label(count: u32) -> String {
    if count == 1 {
        m::one_day()
    } else {
        m::days_count(count)
    }
}

fn kept_reason(reason: KeptReason) -> String {
    match reason {
        KeptReason::Changes => m::merged_kept_changes(),
        KeptReason::Working => m::merged_kept_working(),
        KeptReason::Terminal => m::merged_kept_terminal(),
        KeptReason::InUse => m::merged_kept_in_use(),
        KeptReason::NewCommits => m::merged_kept_new_commits(),
        KeptReason::Failed => m::merged_kept_failed(),
    }
}

/// One line per kept worktree, with why it stayed.
fn kept_tooltip(lines: Vec<String>) -> Tooltip {
    Tooltip::element(move |_, _| {
        div()
            .flex()
            .flex_col()
            .children(lines.iter().map(|line| div().child(line.clone())))
    })
    .max_width(rems(28.))
}

/// When the last sweep of merged worktrees ran and what it removed; what it kept goes apart.
fn cleanup_summary(cleanup: &MergedCleanup, now: DateTime<Utc>) -> String {
    let ran = DateTime::<Utc>::from(cleanup.at).to_rfc3339();
    let last_run = m::merged_cleanup_last_run(crate::sections::relative_time(&ran, now));
    match (cleanup.removed.len(), cleanup.kept.len()) {
        (0, 0) => format!("{last_run} · {}", m::merged_cleanup_nothing()),
        (removed, _) => format!("{last_run} · {}", m::merged_cleanup_removed(removed)),
    }
}

fn workspace_label(workspace: NewThreadWorkspace) -> String {
    workspace_option(workspace == NewThreadWorkspace::NewWorktree).1
}

impl SettingsView {
    pub(super) fn threads(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let settings = self.settings(cx).clone();
        let mut page =
            vec![
                SettingsGroup::new()
                    .title(m::settings_composer())
                    .child(
                        SettingsRow::new(m::plan_mode_setting())
                            .description(m::plan_mode_setting_description())
                            .control(
                                Switch::new("plan-mode-enabled")
                                    .checked(settings.plan_mode_enabled)
                                    .on_change(self.toggler(cx, |settings, on| {
                                        settings.plan_mode_enabled = on
                                    })),
                            ),
                    )
                    .into_any_element(),
                self.new_threads(cx),
                self.new_projects(cx),
                self.thread_states(cx),
                self.interrupted_threads(cx),
                self.snooze_options.clone().into_any_element(),
                self.thread_titles(cx),
            ];
        page.extend(self.save_error(cx));
        page
    }

    fn new_threads(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let defaults = self.settings(cx).new_threads.clone();
        let picker = self.new_thread_model_picker(cx);

        let access = defaults.access_mode;
        let access = Select::new(
            "new-thread-access",
            access.map_or_else(m::last_used, access_label),
        )
        .align(Align::End)
        .menu_width(px(288.))
        .option(
            SelectOption::new(
                "new-thread-access-last-used",
                m::last_used(),
                access.is_none(),
            )
            .description(m::last_used_description())
            .on_select(self.setter(cx, |settings| settings.new_threads.access_mode = None)),
        )
        .options(ACCESS_MODES.map(|mode| {
            SelectOption::new(
                SharedString::from(format!("new-thread-access-{mode:?}")),
                access_label(mode),
                access == Some(mode),
            )
            .description(access_description(mode))
            .on_select(self.setter(cx, move |settings| {
                settings.new_threads.access_mode = Some(mode)
            }))
        }));

        let place = defaults.workspace;
        let workspace = Select::new(
            "new-thread-workspace",
            place.map_or_else(m::last_used, workspace_label),
        )
        .align(Align::End)
        .option(
            SelectOption::new(
                "new-thread-workspace-last-used",
                m::last_used(),
                place.is_none(),
            )
            .on_select(self.setter(cx, |settings| settings.new_threads.workspace = None)),
        )
        .options(
            [NewThreadWorkspace::Local, NewThreadWorkspace::NewWorktree].map(|option| {
                SelectOption::new(
                    SharedString::from(format!("new-thread-workspace-{option:?}")),
                    workspace_label(option),
                    place == Some(option),
                )
                .on_select(self.setter(cx, move |settings| {
                    settings.new_threads.workspace = Some(option)
                }))
            }),
        );

        SettingsGroup::new()
            .title(m::settings_new_threads())
            .child(
                SettingsRow::new(m::new_thread_model_setting())
                    .description(m::new_thread_model_setting_description())
                    .control(picker),
            )
            .child(
                SettingsRow::new(m::new_thread_access_setting())
                    .description(m::new_thread_access_setting_description())
                    .control(access),
            )
            .child(
                SettingsRow::new(m::new_thread_workspace_setting())
                    .description(m::new_thread_workspace_setting_description())
                    .control(workspace),
            )
            .into_any_element()
    }

    fn new_projects(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let general = &self.settings(cx).general;
        let host_folders = general.clone_into_host_folders;
        let browse = Button::new("browse-projects-folder")
            .label(m::browse())
            .variant(ButtonVariant::Outline)
            .size(ButtonSize::Sm)
            .on_click(cx.listener(|this, _, window, cx| this.browse_projects_folder(window, cx)));
        SettingsGroup::new()
            .title(m::settings_new_projects())
            .child(
                SettingsRow::new(m::projects_folder())
                    .description(m::projects_folder_description())
                    .control(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(field(&self.projects_folder, px(224.)))
                            .child(browse),
                    ),
            )
            .child(
                SettingsRow::new(m::clone_into_host_folders())
                    .description(m::clone_into_host_folders_description())
                    .control(
                        Switch::new("settings-clone-into-host-folders")
                            .checked(host_folders)
                            .on_change(self.toggler(cx, |settings, on| {
                                settings.general.clone_into_host_folders = on
                            })),
                    ),
            )
            .into_any_element()
    }

    fn thread_states(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let current = self.settings(cx).general.inactive_after_days;
        let mut days = INACTIVE_DAYS.to_vec();
        if !days.contains(&current) {
            days.push(current);
            days.sort_unstable();
        }
        let inactive = Select::new("settings-inactive-after", days_label(current))
            .min_width(px(128.))
            .options(days.into_iter().map(|count| {
                SelectOption::new(
                    ("inactive-days", count as usize),
                    days_label(count),
                    count == current,
                )
                .on_select(self.setter(cx, move |settings| {
                    settings.general.inactive_after_days = count
                }))
            }));
        SettingsGroup::new()
            .title(m::settings_thread_states())
            .child(
                SettingsRow::new(m::inactive_after())
                    .description(m::inactive_after_no_pull_request_description())
                    .control(inactive),
            )
            .child(self.merged_worktrees_row(cx))
            .into_any_element()
    }

    fn merged_worktrees_row(&mut self, cx: &mut Context<Self>) -> SettingsRow {
        let on = self.settings(cx).general.delete_merged_worktrees;
        let workspace = self.workspace.read(cx);
        let last = workspace.merged_cleanup.as_ref().map(|cleanup| {
            let kept = cleanup
                .kept
                .iter()
                .map(|(id, reason)| {
                    let title = workspace
                        .model
                        .thread(id)
                        .map_or_else(|| id.clone(), |thread| thread.title.clone());
                    m::merged_kept_line(kept_reason(*reason), title)
                })
                .collect::<Vec<_>>();
            (cleanup_summary(cleanup, Utc::now()), kept)
        });
        let colors = cx.hoshi().clone();
        let cleaning = self.cleaning_merged;
        SettingsRow::new(m::delete_merged_worktrees_setting())
            .description(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(m::delete_merged_worktrees_setting_description())
                    .when_some(last, |this, (summary, kept)| {
                        this.child(
                            div()
                                .debug_selector(|| "merged-cleanup-summary".into())
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(summary)
                                .when(!kept.is_empty(), |this| {
                                    this.child("·").child(
                                        div()
                                            .id("merged-cleanup-kept")
                                            .underline()
                                            .text_color(colors.foreground)
                                            .child(m::merged_cleanup_kept(kept.len()))
                                            .with_tooltip(kept_tooltip(kept)),
                                    )
                                }),
                        )
                    }),
            )
            .control(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("settings-clean-up-merged")
                            .label(m::merged_cleanup_now())
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Ghost)
                            .loading(cleaning)
                            .disabled(cleaning)
                            .on_click(cx.listener(|this, _, _, cx| this.clean_up_merged(cx))),
                    )
                    .child(
                        Switch::new("settings-delete-merged-worktrees")
                            .checked(on)
                            .on_change(self.toggler(cx, |settings, on| {
                                settings.general.delete_merged_worktrees = on
                            })),
                    ),
            )
    }

    fn clean_up_merged(&mut self, cx: &mut Context<Self>) {
        if self.cleaning_merged {
            return;
        }
        self.cleaning_merged = true;
        cx.notify();
        let engine = self.workspace.read(cx).engine.clone();
        cx.spawn(async move |this, cx| {
            cx.background_spawn(async move { engine.delete_merged_worktrees() })
                .await;
            this.update(cx, |this, cx| {
                this.cleaning_merged = false;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn interrupted_threads(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let general = &self.settings(cx).general;
        let (after_restart, after_limit, snooze_limited) = (
            general.continue_after_restart,
            general.resume_after_usage_limit,
            general.snooze_until_limit_resets,
        );
        SettingsGroup::new()
            .title(m::settings_interrupted_threads())
            .child(
                SettingsRow::new(m::continue_after_restart_setting())
                    .description(m::continue_after_restart_setting_description())
                    .control(
                        Switch::new("settings-continue-after-restart")
                            .checked(after_restart)
                            .on_change(self.toggler(cx, |settings, on| {
                                settings.general.continue_after_restart = on
                            })),
                    ),
            )
            .child(
                SettingsRow::new(m::resume_after_usage_limit_setting())
                    .description(m::resume_after_usage_limit_setting_description())
                    .control(
                        Switch::new("settings-resume-after-usage-limit")
                            .checked(after_limit)
                            .on_change(self.toggler(cx, |settings, on| {
                                settings.general.resume_after_usage_limit = on
                            })),
                    ),
            )
            .child(
                SettingsRow::new(m::snooze_until_limit_resets_setting())
                    .description(m::snooze_until_limit_resets_setting_description())
                    .control(
                        Switch::new("settings-snooze-until-limit-resets")
                            .checked(snooze_limited)
                            .on_change(self.toggler(cx, |settings, on| {
                                settings.general.snooze_until_limit_resets = on
                            })),
                    ),
            )
            .into_any_element()
    }

    fn thread_titles(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let titles = self.settings(cx).thread_titles.clone();
        let picker = self.title_model_picker(titles.generate, cx);
        SettingsGroup::new()
            .title(m::settings_thread_titles())
            .child(
                SettingsRow::new(m::thread_titles_generate())
                    .description(m::thread_titles_generate_description())
                    .control(
                        Switch::new("thread-titles-generate")
                            .checked(titles.generate)
                            .on_change(
                                self.toggler(cx, |settings, on| {
                                    settings.thread_titles.generate = on
                                }),
                            ),
                    ),
            )
            .child(
                SettingsRow::new(m::thread_titles_model())
                    .description(m::thread_titles_model_description())
                    .control(picker),
            )
            .child(
                SettingsRow::new(m::thread_titles_instructions())
                    .description(m::thread_titles_instructions_description())
                    .child(text_area(&self.title_instructions, !titles.generate)),
            )
            .into_any_element()
    }

    /// The model that names threads, picked as in the composer.
    fn title_model_picker(&mut self, enabled: bool, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.hoshi().clone();
        let titles = self.settings(cx).thread_titles.clone();
        let workspace = self.workspace.read(cx);
        let catalogs = workspace.models.clone();
        let favorites = workspace.settings.favorite_models.clone();
        let provider = titles
            .provider
            .clone()
            .unwrap_or_else(hoshi_core::default_provider);
        let name = match &titles.model {
            None => m::thread_titles_model_default(),
            Some(slug) => workspace
                .chosen_model(&provider, Some(slug))
                .map_or_else(|| slug.clone(), |model| model.name.clone()),
        };
        let selected = titles.model.clone().map(|slug| (provider.clone(), slug));
        let list = self.picker_list(catalogs, favorites, &provider, selected, cx);
        let trigger =
            picker_trigger("title-model-trigger", &provider, name, None, &colors).map(|trigger| {
                if enabled {
                    trigger
                        .cursor_pointer()
                        .hover(|style| style.bg(colors.muted))
                } else {
                    trigger.opacity(0.5)
                }
            });
        if !enabled {
            return trigger.into_any_element();
        }
        let this = cx.weak_entity();
        self.picker_popover(
            "title-model-menu",
            ModelSetting::ThreadTitles,
            trigger,
            cx,
            move |_, cx| list.render(this.clone(), &cx.hoshi().clone()),
        )
    }

    /// The model new threads start with, or "Last used" to keep the last thread's.
    fn new_thread_model_picker(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.hoshi().clone();
        let fixed = self.settings(cx).new_threads.model.clone();
        let workspace = self.workspace.read(cx);
        let catalogs = workspace.models.clone();
        let favorites = workspace.settings.favorite_models.clone();
        let provider = fixed.as_ref().map_or_else(
            || workspace.last_used.model.provider.clone(),
            |fixed| fixed.provider.clone(),
        );
        let model = fixed
            .as_ref()
            .and_then(|fixed| workspace.chosen_model(&fixed.provider, Some(&fixed.model)))
            .cloned();
        let choice = fixed.as_ref().map(|fixed| ModelChoice {
            effort: effective_effort(model.as_ref(), fixed.effort),
            ..fixed_choice(fixed)
        });
        let name = match (&fixed, &model) {
            (None, _) => m::last_used(),
            (Some(_), Some(model)) => model.name.clone(),
            (Some(fixed), None) => fixed.model.clone(),
        };
        let selected = fixed
            .as_ref()
            .map(|fixed| (fixed.provider.clone(), fixed.model.clone()));
        let list = self.picker_list(catalogs, favorites, &provider, selected, cx);
        let effort = choice.as_ref().and_then(|choice| choice.effort);
        let trigger = picker_trigger("new-thread-model-trigger", &provider, name, effort, &colors)
            .cursor_pointer()
            .hover(|style| style.bg(colors.muted));
        let last_used = fixed.is_none();
        let this = cx.weak_entity();
        self.picker_popover(
            "new-thread-model-menu",
            ModelSetting::NewThreads,
            trigger,
            cx,
            move |_, cx| {
                let colors = cx.hoshi().clone();
                let pick_last_used = this.clone();
                let reset = div()
                    .id("new-thread-model-last-used")
                    .debug_selector(|| "new-thread-model-last-used".into())
                    .flex()
                    .items_center()
                    .gap_2()
                    .mx_1()
                    .mt_1()
                    .px_2()
                    .py_1p5()
                    .rounded_lg()
                    .cursor_pointer()
                    .text_sm()
                    .hover(|style| style.bg(colors.accent.opacity(0.6)))
                    .when(last_used, |row| row.bg(colors.accent))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().font_weight(FontWeight::MEDIUM).child(m::last_used()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(colors.muted_foreground)
                                    .child(m::last_used_description()),
                            ),
                    )
                    .when(last_used, |row| {
                        row.child(Icon::new(IconName::Check).size_4().flex_none())
                    })
                    .on_click(move |_, _, cx| {
                        pick_last_used
                            .update(cx, |this, cx| {
                                this.model_picker.open = None;
                                this.change(cx, |settings| settings.new_threads.model = None);
                                cx.notify();
                            })
                            .ok();
                    });
                let options = model
                    .as_ref()
                    .zip(choice.as_ref())
                    .and_then(|(model, choice)| {
                        model_options(model, choice, this.clone(), &colors)
                    });
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .pb_1()
                            .border_b_1()
                            .border_color(colors.border)
                            .child(reset),
                    )
                    .child(list.render(this.clone(), &colors))
                    .children(options)
            },
        )
    }

    fn picker_list(
        &self,
        catalogs: Vec<hoshi_core::ProviderModels>,
        favorites: Vec<hoshi_core::FavoriteModel>,
        provider: &str,
        selected: Option<(String, String)>,
        cx: &App,
    ) -> PickerList {
        let tab = self
            .model_picker
            .tab
            .clone()
            .unwrap_or_else(|| default_tab(&catalogs, &favorites, provider));
        let search = self.model_picker.search.clone();
        PickerList {
            query: search.read(cx).value().to_string(),
            catalogs,
            favorites,
            tab,
            show_legacy: self.model_picker.show_legacy,
            selected,
            search,
        }
    }

    fn picker_popover<E: IntoElement + 'static>(
        &self,
        id: &'static str,
        setting: ModelSetting,
        trigger: Stateful<Div>,
        cx: &mut Context<Self>,
        content: impl FnOnce(&mut Window, &mut App) -> E + 'static,
    ) -> AnyElement {
        let muted = cx.hoshi().muted;
        let focus = self.model_picker.search.read(cx).focus_handle(cx);
        Popover::new(id)
            .side(Side::Bottom)
            .align(Align::End)
            .modal(false)
            .w_96()
            .p_0()
            .gap_0()
            .rounded_xl()
            .overflow_hidden()
            .initial_focus(PopoverFocus::Handle(focus))
            .open(self.model_picker.open == Some(setting))
            .on_open_change(
                cx.listener(move |this, change: &PopoverChange, window, cx| {
                    this.open_model_picker(setting, change.open, window, cx)
                }),
            )
            .trigger(move |open| trigger.when(open, |trigger| trigger.bg(muted)))
            .content(move |_, window, cx| content(window, cx))
            .into_any_element()
    }

    pub(super) fn open_model_picker(
        &mut self,
        setting: ModelSetting,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if open {
            self.model_picker.tab = None;
            self.model_picker
                .search
                .update(cx, |search, cx| search.set_value("", window, cx));
            self.model_picker.open = Some(setting);
        } else if self.model_picker.open == Some(setting) {
            self.model_picker.open = None;
        }
        cx.notify();
    }

    fn browse_projects_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = folder_picker::start_folder([
            self.workspace
                .read(cx)
                .settings
                .general
                .projects_folder
                .clone(),
            crate::palette::add_project::home(),
        ]);
        let picked = folder_picker::pick_folder(start, None, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let Some(path) = picked.await else {
                return;
            };
            let path = path.to_string_lossy().to_string();
            this.update_in(cx, |this, window, cx| {
                this.projects_folder
                    .update(cx, |field, cx| field.set_value(path.clone(), window, cx));
                this.change(cx, |settings| settings.general.projects_folder = Some(path));
            })
            .ok();
        })
        .detach();
    }
}

fn fixed_choice(fixed: &NewThreadModel) -> ModelChoice {
    ModelChoice {
        provider: fixed.provider.clone(),
        model: Some(fixed.model.clone()),
        effort: fixed.effort,
        fast_mode: fixed.fast_mode,
        ultracode: fixed.ultracode,
    }
}

fn picker_trigger(
    id: &'static str,
    provider: &str,
    name: String,
    effort: Option<Effort>,
    colors: &Colors,
) -> Stateful<Div> {
    let (mark, tint) = (provider_mark(provider), provider_tint(provider, colors));
    div()
        .id(id)
        .debug_selector(move || id.into())
        .flex()
        .items_center()
        .gap_1p5()
        .h_8()
        .px_2p5()
        .rounded_lg()
        .border_1()
        .border_color(colors.border)
        .text_sm()
        .child(svg().path(mark).size_3p5().flex_none().text_color(tint))
        .child(div().font_weight(FontWeight::MEDIUM).child(name))
        .when_some(effort, |trigger, effort| {
            trigger.child(
                div()
                    .text_color(colors.muted_foreground)
                    .child(effort_label(effort)),
            )
        })
        .child(Icon::new(IconName::ChevronDown).size_3p5())
}

/// Which setting the open model picker picks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ModelSetting {
    ThreadTitles,
    NewThreads,
}

/// A model picker's state while it is open.
pub(super) struct ModelPicker {
    pub search: Entity<InputState>,
    pub tab: Option<ModelTab>,
    pub show_legacy: bool,
    pub open: Option<ModelSetting>,
}

impl PickerHost for SettingsView {
    fn picker_search(&self) -> Entity<InputState> {
        self.model_picker.search.clone()
    }

    fn set_picker_tab(&mut self, tab: ModelTab) {
        self.model_picker.tab = Some(tab);
    }

    fn toggle_picker_legacy(&mut self) {
        self.model_picker.show_legacy = !self.model_picker.show_legacy;
    }

    fn picker_workspace(&self) -> Entity<Workspace> {
        self.workspace.clone()
    }

    fn pick_model(
        &mut self,
        provider: String,
        slug: String,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.model_picker.open.take() {
            Some(ModelSetting::ThreadTitles) => self.change(cx, |settings| {
                settings.thread_titles.provider = Some(provider);
                settings.thread_titles.model = Some(slug);
            }),
            Some(ModelSetting::NewThreads) => {
                let workspace = self.workspace.read(cx);
                let kept = self.settings(cx).new_threads.model.clone();
                let info = workspace.chosen_model(&provider, Some(&slug));
                let effort = kept
                    .as_ref()
                    .and_then(|kept| kept.effort)
                    .filter(|effort| info.is_none_or(|info| info.efforts.contains(effort)));
                let fixed = NewThreadModel {
                    fast_mode: kept.as_ref().is_some_and(|kept| kept.fast_mode)
                        && info.is_some_and(|info| info.fast_mode),
                    ultracode: kept.as_ref().is_some_and(|kept| kept.ultracode)
                        && info.is_some_and(|info| info.ultracode),
                    provider,
                    model: slug,
                    effort,
                };
                self.change(cx, |settings| settings.new_threads.model = Some(fixed));
            }
            None => {}
        }
        cx.notify();
    }

    fn change_choice(&mut self, change: impl FnOnce(&mut ModelChoice), cx: &mut Context<Self>) {
        let Some(fixed) = self.settings(cx).new_threads.model.clone() else {
            return;
        };
        let mut choice = fixed_choice(&fixed);
        change(&mut choice);
        let Some(model) = choice.model else {
            return;
        };
        let next = NewThreadModel {
            provider: choice.provider,
            model,
            effort: choice.effort,
            fast_mode: choice.fast_mode,
            ultracode: choice.ultracode,
        };
        self.change(cx, |settings| settings.new_threads.model = Some(next));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::cleanup_summary;
    use chrono::{DateTime, Duration, Utc};
    use hoshi_engine::{KeptReason, MergedCleanup};
    use std::time::{Duration as StdDuration, SystemTime};

    #[test]
    fn the_last_cleanup_says_when_it_ran_and_what_it_removed() {
        let at = SystemTime::UNIX_EPOCH + StdDuration::from_secs(1_800_000_000);
        let now = DateTime::<Utc>::from(at) + Duration::minutes(12);
        let mut cleanup = MergedCleanup {
            at,
            removed: Vec::new(),
            kept: Vec::new(),
        };
        assert_eq!(
            cleanup_summary(&cleanup, now),
            "Last run: 12m ago · nothing to remove"
        );
        cleanup.kept.push(("a".into(), KeptReason::Changes));
        assert_eq!(
            cleanup_summary(&cleanup, now),
            "Last run: 12m ago · 0 removed"
        );
        cleanup.removed = vec!["b".into(), "c".into()];
        assert_eq!(
            cleanup_summary(&cleanup, now),
            "Last run: 12m ago · 2 removed"
        );
    }
}
