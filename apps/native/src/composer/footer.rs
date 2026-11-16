//! Under the composer: where the thread runs, and what the next turn runs with: plan mode,
//! access, model, effort and speed.

use super::Composer;
use super::model_picker::PickerHost;
use crate::m;
use crate::state::ModelChoice;
use gpui_kit::component::{Icon, Selectable as _};
use gpui_kit::*;
use hoshi_core::{AccessMode, Command, Effort, ModelInfo};
use hoshi_ui::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FooterMenu {
    Workspace,
    Access,
    Model,
}

/// What the footer sets: the open thread's choices, or the draft's.
#[derive(Clone, Debug)]
pub(super) struct Target {
    thread_id: Option<String>,
    project_id: Option<String>,
    pub access_mode: AccessMode,
    pub plan_mode: bool,
    pub choice: ModelChoice,
    worktree: bool,
}

impl FooterMenu {
    /// `HOSHI_NATIVE_MENU=access|model` opens that menu at start, to look at it without a mouse.
    pub(super) fn from_env() -> Option<Self> {
        match std::env::var("HOSHI_NATIVE_MENU").ok()?.as_str() {
            "workspace" => Some(Self::Workspace),
            "access" => Some(Self::Access),
            "model" => Some(Self::Model),
            _ => None,
        }
    }
}

pub(crate) const ACCESS_MODES: [AccessMode; 4] = [
    AccessMode::Ask,
    AccessMode::AcceptEdits,
    AccessMode::Auto,
    AccessMode::FullAccess,
];

fn access_icon(mode: AccessMode) -> IconName {
    match mode {
        AccessMode::Ask => IconName::ShieldUser,
        AccessMode::AcceptEdits => IconName::Pencil,
        AccessMode::Auto => IconName::WandSparkles,
        AccessMode::FullAccess => IconName::LockOpen,
    }
}

pub(crate) fn access_label(mode: AccessMode) -> String {
    match mode {
        AccessMode::Ask => m::access_ask(),
        AccessMode::AcceptEdits => m::access_accept_edits(),
        AccessMode::Auto => m::access_auto(),
        AccessMode::FullAccess => m::access_full(),
    }
}

pub(crate) fn access_description(mode: AccessMode) -> String {
    match mode {
        AccessMode::Ask => m::access_ask_description(),
        AccessMode::AcceptEdits => m::access_accept_edits_description(),
        AccessMode::Auto => m::access_auto_description(),
        AccessMode::FullAccess => m::access_full_description(),
    }
}

pub fn effort_label(effort: Effort) -> String {
    match effort {
        Effort::Low => m::effort_low(),
        Effort::Medium => m::effort_medium(),
        Effort::High => m::effort_high(),
        Effort::Xhigh => m::effort_xhigh(),
        Effort::Max => m::effort_max(),
    }
}

fn effort_short_label(effort: Effort) -> String {
    match effort {
        Effort::Medium => m::effort_medium_short(),
        Effort::Xhigh => m::effort_xhigh_short(),
        other => effort_label(other),
    }
}

/// The effort a model runs at: the one chosen when it supports it, else its default.
pub fn effective_effort(model: Option<&ModelInfo>, effort: Option<Effort>) -> Option<Effort> {
    match (model, effort) {
        (Some(model), Some(effort)) if model.efforts.contains(&effort) => Some(effort),
        (Some(model), _) => model.default_effort,
        (None, _) => None,
    }
}

/// A change to the choice, kept to what the model supports.
pub fn fit_choice(next: ModelChoice, model: Option<&ModelInfo>) -> ModelChoice {
    let Some(model) = model else {
        return next;
    };
    ModelChoice {
        effort: next.effort.filter(|effort| model.efforts.contains(effort)),
        fast_mode: next.fast_mode && model.fast_mode,
        ultracode: next.ultracode && model.ultracode,
        provider: next.provider,
        model: next.model,
    }
}

impl Composer {
    pub(super) fn target(&self, cx: &App) -> Option<Target> {
        let workspace = self.workspace.read(cx);
        if let Some(id) = workspace.open_thread_id(cx) {
            let thread = workspace.model.thread(&id)?;
            return Some(Target {
                thread_id: Some(thread.id.clone()),
                project_id: thread.project_id.clone(),
                access_mode: thread.access_mode,
                plan_mode: thread.plan_mode,
                choice: match thread.pending_model.as_deref() {
                    Some(pending) => ModelChoice {
                        provider: pending.provider.clone(),
                        model: pending.model.clone(),
                        effort: pending.effort,
                        fast_mode: pending.fast_mode,
                        ultracode: pending.ultracode,
                    },
                    None => ModelChoice {
                        provider: thread.provider.clone(),
                        model: thread.model.clone(),
                        effort: thread.effort,
                        fast_mode: thread.fast_mode,
                        ultracode: thread.ultracode,
                    },
                },
                worktree: thread.worktree_path.is_some(),
            });
        }
        let draft = workspace.draft.as_ref()?;
        let choices = &workspace.choices;
        Some(Target {
            thread_id: None,
            project_id: draft.project_id.clone(),
            access_mode: choices.access_mode,
            plan_mode: choices.plan_mode,
            choice: choices.model.clone(),
            worktree: choices.worktree,
        })
    }

    pub(super) fn dispatch(&mut self, command: Command, cx: &mut Context<Self>) {
        let engine = self.workspace.read(cx).engine.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move { engine.dispatch(command) })
                .await;
            if let Err(error) = result {
                this.update(cx, |this, cx| {
                    this.error = Some(super::command_error_message(&error).into());
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    pub(super) fn set_access(
        &mut self,
        access_mode: AccessMode,
        plan_mode: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.target(cx) else {
            return;
        };
        self.workspace.update(cx, |workspace, cx| {
            workspace.set_choices(
                |choices| {
                    choices.access_mode = access_mode;
                    choices.plan_mode = plan_mode;
                },
                cx,
            )
        });
        if let Some(thread_id) = target.thread_id {
            self.dispatch(
                Command::ThreadAccessSet {
                    thread_id,
                    access_mode,
                    plan_mode,
                },
                cx,
            );
        }
        cx.notify();
    }

    pub(super) fn set_model(
        &mut self,
        change: impl FnOnce(&mut ModelChoice),
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.target(cx) else {
            return;
        };
        let mut next = target.choice;
        change(&mut next);
        let model = self
            .workspace
            .read(cx)
            .chosen_model(&next.provider, next.model.as_deref());
        let next = fit_choice(next, model);
        self.workspace.update(cx, |workspace, cx| {
            workspace.set_choices(|choices| choices.model = next.clone(), cx)
        });
        if let Some(thread_id) = target.thread_id {
            self.dispatch(
                Command::ThreadModelSet {
                    thread_id,
                    provider: next.provider,
                    model: next.model,
                    effort: next.effort,
                    fast_mode: next.fast_mode,
                    ultracode: next.ultracode,
                },
                cx,
            );
        }
        cx.notify();
    }

    fn set_worktree(&mut self, worktree: bool, cx: &mut Context<Self>) {
        self.workspace.update(cx, |workspace, cx| {
            workspace.set_choices(|choices| choices.worktree = worktree, cx)
        });
        cx.notify();
    }

    pub(super) fn set_menu(&mut self, menu: FooterMenu, open: bool, cx: &mut Context<Self>) {
        if open {
            self.menu = Some(menu);
        } else if self.menu == Some(menu) {
            self.menu = None;
        }
        cx.notify();
    }

    pub(super) fn render_footer(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.hoshi().clone();
        let target = self.target(cx);
        let workspace = self.workspace.read(cx);
        let place = target.as_ref().map(|target| match &target.project_id {
            None => (IconName::MessageCircle, m::chat()),
            Some(project) => (
                IconName::Folder,
                workspace
                    .model
                    .project(project)
                    .map(|project| project.title.clone())
                    .unwrap_or_default(),
            ),
        });
        let plan_enabled = workspace.settings.plan_mode_enabled;

        let mut controls = div().ml_auto().flex().flex_none().items_center().gap_1();
        if let Some(target) = &target {
            if target.thread_id.is_none() && target.project_id.is_some() {
                controls = controls.child(self.workspace_menu(target.worktree, cx));
            }
            if plan_enabled || target.plan_mode {
                let (access, plan) = (target.access_mode, target.plan_mode);
                controls = controls.child(
                    Button::new("plan-mode")
                        .icon(IconName::ListTodo)
                        .label(m::plan_mode())
                        .size(ButtonSize::Xs)
                        .variant(ButtonVariant::Ghost)
                        .selected(plan)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.set_access(access, !plan, cx)),
                        ),
                );
            }
            controls = controls
                .child(self.access_menu(target, cx))
                .children(self.render_compaction_chip(cx))
                .children(self.render_context_meter(cx))
                .child(self.model_menu(target, cx));
        }

        div()
            .flex()
            .items_center()
            .gap_3()
            .px_2()
            .pt_1p5()
            .text_xs()
            .text_color(colors.muted_foreground)
            .when_some(place, |row, (icon, label)| {
                row.child(
                    div()
                        .flex()
                        .min_w_0()
                        .items_center()
                        .gap_1p5()
                        .child(Icon::new(icon).size_3p5().flex_none())
                        .child(div().truncate().child(label)),
                )
            })
            .when_some(
                self.notice.clone().filter(|_| self.error.is_none()),
                |row, notice| {
                    row.child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(colors.muted_foreground)
                            .child(notice),
                    )
                },
            )
            .when_some(self.error.clone(), |row, error| {
                row.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(colors.destructive)
                        .child(error),
                )
            })
            .child(controls)
            .into_any_element()
    }

    fn workspace_menu(&mut self, worktree: bool, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.weak_entity();
        let (icon, label) = workspace_option(worktree);
        Popover::new("workspace-menu")
            .side(Side::Top)
            .align(Align::End)
            .w(px(176.))
            .open(self.menu == Some(FooterMenu::Workspace))
            .on_open_change(cx.listener(|this, change: &PopoverChange, _, cx| {
                this.set_menu(FooterMenu::Workspace, change.open, cx)
            }))
            .trigger(move |open| {
                Button::new("workspace-trigger")
                    .icon(icon)
                    .label(label)
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Ghost)
                    .selected(open)
            })
            .menu(move |menu, _, _| {
                let options = [false, true].into_iter().map(|option| {
                    let this = this.clone();
                    let (icon, label) = workspace_option(option);
                    MenuOption::radio(
                        ("workspace-option", option as usize),
                        label,
                        option == worktree,
                    )
                    .icon(icon)
                    .close_on_click(true)
                    .on_select(move |_, cx| {
                        this.update(cx, |this, cx| this.set_worktree(option, cx))
                            .ok();
                    })
                });
                menu.heading(m::workspace_label()).items(options)
            })
            .into_any_element()
    }

    fn access_menu(&mut self, target: &Target, cx: &mut Context<Self>) -> AnyElement {
        let current = target.access_mode;
        let plan = target.plan_mode;
        let this = cx.weak_entity();
        Popover::new("access-menu")
            .side(Side::Top)
            .align(Align::End)
            .w_72()
            .open(self.menu == Some(FooterMenu::Access))
            .on_open_change(cx.listener(|this, change: &PopoverChange, _, cx| {
                this.set_menu(FooterMenu::Access, change.open, cx)
            }))
            .trigger(move |open| {
                Button::new("access-trigger")
                    .icon(access_icon(current))
                    .label(access_label(current))
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Ghost)
                    .selected(open)
            })
            .menu(move |menu, _, _| {
                let options = ACCESS_MODES.into_iter().map(|mode| {
                    let this = this.clone();
                    MenuOption::radio(
                        SharedString::from(format!("access-{mode:?}")),
                        access_label(mode),
                        mode == current,
                    )
                    .icon(access_icon(mode))
                    .description(access_description(mode))
                    .close_on_click(true)
                    .on_select(move |_, cx| {
                        this.update(cx, |this, cx| this.set_access(mode, plan, cx))
                            .ok();
                    })
                });
                menu.heading(m::access_label()).items(options)
            })
            .into_any_element()
    }
}

pub(crate) fn workspace_option(worktree: bool) -> (IconName, String) {
    if worktree {
        (IconName::GitFork, m::workspace_new_worktree())
    } else {
        (IconName::Monitor, m::workspace_local())
    }
}

pub(super) fn tag(text: String, colors: &Colors) -> Div {
    div()
        .flex_none()
        .rounded_sm()
        .px_1()
        .bg(colors.primary)
        .text_color(colors.primary_foreground)
        .text_size(px(10.))
        .font_weight(FontWeight::MEDIUM)
        .child(text)
}

pub(crate) fn model_options<H: PickerHost>(
    model: &ModelInfo,
    choice: &ModelChoice,
    this: WeakEntity<H>,
    colors: &Colors,
) -> Option<AnyElement> {
    if model.efforts.is_empty() && !model.fast_mode && !model.ultracode {
        return None;
    }
    let levels = model.efforts.len();
    let efforts = model.efforts.iter().enumerate().fold(
        Segmented::new("effort"),
        |segmented, (index, effort)| {
            let (this, effort) = (this.clone(), *effort);
            segmented.segment(
                Segment::new(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_1()
                        .pt_1p5()
                        .pb_1()
                        .text_size(px(10.))
                        .child(effort_meter(
                            index + 1,
                            levels,
                            if choice.effort == Some(effort) {
                                colors.foreground
                            } else {
                                colors.muted_foreground
                            },
                        ))
                        .child(effort_short_label(effort)),
                )
                .selected(choice.effort == Some(effort))
                .on_click(move |_, _, cx| {
                    this.update(cx, |this, cx| {
                        this.change_choice(|choice| choice.effort = Some(effort), cx)
                    })
                    .ok();
                }),
            )
        },
    );
    let effort_header = div()
        .mb_2()
        .flex()
        .items_center()
        .justify_between()
        .text_xs()
        .text_color(colors.muted_foreground)
        .child(m::effort_label())
        .when_some(choice.effort, |row, effort| {
            row.child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .when(Some(effort) == model.default_effort, |row| {
                        row.child(
                            div()
                                .rounded_sm()
                                .border_1()
                                .border_color(colors.border)
                                .px_1()
                                .text_size(px(10.))
                                .child(m::effort_default_tag()),
                        )
                    })
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(colors.foreground)
                            .child(effort_label(effort)),
                    ),
            )
        });

    let speeds = model.fast_mode.then(|| {
        [(false, m::speed_standard()), (true, m::speed_fast())]
            .into_iter()
            .fold(Segmented::new("speed"), |segmented, (fast, label)| {
                let this = this.clone();
                let selected = choice.fast_mode == fast;
                segmented.segment(
                    Segment::new(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .h_6()
                            .text_xs()
                            .when(fast, |row| {
                                row.child(
                                    Icon::new(IconName::Zap).size_3p5().when(selected, |icon| {
                                        icon.text_color(colors.provider_claude)
                                    }),
                                )
                            })
                            .child(label),
                    )
                    .selected(selected)
                    .on_click(move |_, _, cx| {
                        this.update(cx, |this, cx| {
                            this.change_choice(|choice| choice.fast_mode = fast, cx)
                        })
                        .ok();
                    }),
                )
            })
    });
    let ultracode = model.ultracode.then(|| {
        let on = choice.ultracode;
        let this = this.clone();
        let foreground = colors.foreground;
        div()
            .id("ultracode")
            .flex()
            .items_center()
            .gap_1()
            .h_7()
            .px_2p5()
            .rounded_lg()
            .border_1()
            .border_color(colors.border)
            .cursor_pointer()
            .text_xs()
            .when(on, |row| {
                row.bg(colors.popover)
                    .text_color(colors.foreground)
                    .font_weight(FontWeight::MEDIUM)
            })
            .when(!on, |row| {
                row.text_color(colors.muted_foreground)
                    .hover(move |style| style.text_color(foreground))
            })
            .child(
                Icon::new(IconName::Workflow)
                    .size_3p5()
                    .when(on, |icon| icon.text_color(colors.provider_claude)),
            )
            .child(m::ultracode())
            .on_click(move |_, _, cx| {
                this.update(cx, |this, cx| {
                    this.change_choice(|choice| choice.ultracode = !on, cx)
                })
                .ok();
            })
    });

    let section = div()
        .flex()
        .flex_col()
        .gap_2p5()
        .px_3()
        .pt_2p5()
        .pb_3()
        .border_t_1()
        .border_color(colors.border)
        .bg(colors.muted.opacity(0.4))
        .when(levels > 0, |section| {
            section.child(div().child(effort_header).child(efforts))
        })
        .when(speeds.is_some() || ultracode.is_some(), |section| {
            section.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when_some(speeds, |row, speeds| {
                        row.child(div().flex_1().child(speeds))
                    })
                    .children(ultracode),
            )
        });
    Some(section.into_any_element())
}

/// `level` bars lit out of the model's `levels` effort options.
fn effort_meter(level: usize, levels: usize, color: Hsla) -> Div {
    const HEIGHTS: [f32; 5] = [4., 6., 8., 10., 12.];
    let lit = ((level as f32 / levels as f32) * HEIGHTS.len() as f32).round() as usize;
    div()
        .flex()
        .items_end()
        .gap_px()
        .h_3()
        .children(HEIGHTS.iter().enumerate().map(|(index, height)| {
            div()
                .w_0p5()
                .h(px(*height))
                .rounded_full()
                .bg(color)
                .when(index >= lit, |bar| bar.opacity(0.25))
        }))
}

#[cfg(test)]
mod tests {
    use super::{ModelChoice, effective_effort, fit_choice};
    use hoshi_core::{Effort, ModelInfo};

    fn model(efforts: Vec<Effort>, default_effort: Option<Effort>) -> ModelInfo {
        ModelInfo {
            slug: "m".into(),
            name: "Model".into(),
            short_name: "M".into(),
            legacy: false,
            is_new: false,
            update_required: false,
            efforts,
            default_effort,
            fast_mode: false,
            ultracode: true,
        }
    }

    #[test]
    fn an_effort_the_model_lacks_falls_back_to_its_default() {
        let model = model(vec![Effort::Low, Effort::High], Some(Effort::High));
        assert_eq!(
            effective_effort(Some(&model), Some(Effort::Low)),
            Some(Effort::Low)
        );
        assert_eq!(
            effective_effort(Some(&model), Some(Effort::Max)),
            Some(Effort::High)
        );
        assert_eq!(effective_effort(None, Some(Effort::Max)), None);
    }

    #[test]
    fn a_choice_keeps_only_what_the_model_supports() {
        let model = model(vec![Effort::Low], None);
        let fitted = fit_choice(
            ModelChoice {
                provider: hoshi_core::default_provider(),
                model: Some("m".into()),
                effort: Some(Effort::Max),
                fast_mode: true,
                ultracode: true,
            },
            Some(&model),
        );
        assert_eq!(fitted.effort, None);
        assert!(!fitted.fast_mode);
        assert!(fitted.ultracode);
    }
}
