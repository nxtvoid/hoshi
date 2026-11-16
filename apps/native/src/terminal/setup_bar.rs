//! Above the composer while the project's setup runs in the thread's worktree: how it went,
//! its last lines, and its terminal one click away.

use super::TerminalDrawer;
use crate::m;
use crate::state::Workspace;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::{WorktreeSetupRun, WorktreeSetupStatus};
use hoshi_ui::prelude::*;
use std::collections::HashSet;
use std::time::Duration;

fn headline(run: &WorktreeSetupRun) -> String {
    match run.status {
        WorktreeSetupStatus::Running => m::worktree_setup_running(),
        WorktreeSetupStatus::Succeeded => m::worktree_setup_succeeded(),
        WorktreeSetupStatus::Failed if !run.failed_files.is_empty() => {
            m::worktree_setup_files_failed()
        }
        WorktreeSetupStatus::Failed => match run.exit_code {
            Some(code) => m::worktree_setup_failed_code(code),
            None => m::worktree_setup_failed(),
        },
    }
}

fn files_summary(run: &WorktreeSetupRun) -> String {
    let mut parts = Vec::new();
    if run.copied_files > 0 {
        parts.push(m::worktree_setup_copied(run.copied_files as usize));
    }
    if !run.missing_files.is_empty() {
        parts.push(m::worktree_setup_missing(run.missing_files.join(", ")));
    }
    if !run.failed_files.is_empty() {
        parts.push(m::worktree_setup_copy_failed(run.failed_files.join(", ")));
    }
    parts.join(" · ")
}

pub struct WorktreeSetupBar {
    workspace: Entity<Workspace>,
    terminal: WeakEntity<TerminalDrawer>,
    /// Threads whose finished setup the user put away.
    dismissed: HashSet<String>,
    _observe: Subscription,
}

impl WorktreeSetupBar {
    pub fn new(
        workspace: Entity<Workspace>,
        terminal: &Entity<TerminalDrawer>,
        cx: &mut Context<Self>,
    ) -> Self {
        let observe = cx.observe(&workspace, |this: &mut Self, workspace, cx| {
            let runs = &workspace.read(cx).worktree_setups;
            this.dismissed.retain(|thread_id| {
                runs.get(thread_id)
                    .is_none_or(|run| run.status != WorktreeSetupStatus::Running)
            });
            cx.notify();
        });
        Self {
            workspace,
            terminal: terminal.downgrade(),
            dismissed: HashSet::new(),
            _observe: observe,
        }
    }

    fn show_terminal(&self, thread_id: &str, terminal_id: &str, cx: &mut App) {
        self.terminal
            .update(cx, |drawer, cx| {
                drawer.show_terminal(thread_id, terminal_id, cx)
            })
            .ok();
    }

    fn dismiss(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        self.dismissed.insert(thread_id.to_owned());
        cx.notify();
    }

    /// The run the bar shows for the thread on screen.
    #[cfg(test)]
    pub fn shown(&self, cx: &App) -> Option<WorktreeSetupRun> {
        let workspace = self.workspace.read(cx);
        workspace
            .open_thread_id(cx)
            .filter(|thread_id| !self.dismissed.contains(thread_id))
            .and_then(|thread_id| workspace.worktree_setups.get(&thread_id).cloned())
    }

    #[cfg(test)]
    pub fn click_open_terminal(&mut self, cx: &mut Context<Self>) {
        if let Some(run) = self.shown(cx)
            && let Some(terminal_id) = &run.terminal_id
        {
            self.show_terminal(&run.thread_id, terminal_id, cx);
        }
    }

    #[cfg(test)]
    pub fn click_dismiss(&mut self, cx: &mut Context<Self>) {
        if let Some(run) = self.shown(cx) {
            self.dismiss(&run.thread_id, cx);
        }
    }
}

impl Render for WorktreeSetupBar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let workspace = self.workspace.read(cx);
        let run = workspace
            .open_thread_id(cx)
            .filter(|thread_id| !self.dismissed.contains(thread_id))
            .and_then(|thread_id| workspace.worktree_setups.get(&thread_id).cloned());
        let Some(run) = run else {
            return div().id("worktree-setup");
        };
        let colors = cx.hoshi().clone();
        let running = run.status == WorktreeSetupStatus::Running;
        let failed = run.status == WorktreeSetupStatus::Failed;
        let icon: AnyElement = match run.status {
            WorktreeSetupStatus::Running => Icon::new(IconName::LoaderCircle)
                .size_3p5()
                .flex_none()
                .text_color(colors.muted_foreground)
                .with_animation(
                    "worktree-setup-spin",
                    Animation::new(Duration::from_secs(1)).repeat(),
                    |icon, delta| icon.transform(Transformation::rotate(percentage(delta))),
                )
                .into_any_element(),
            WorktreeSetupStatus::Failed => Icon::new(IconName::TriangleAlert)
                .size_3p5()
                .flex_none()
                .text_color(colors.status_error)
                .into_any_element(),
            WorktreeSetupStatus::Succeeded => Icon::new(IconName::CircleCheck)
                .size_3p5()
                .flex_none()
                .text_color(colors.pr_open)
                .into_any_element(),
        };
        let mut text = headline(&run);
        let shown = text.len();
        if run.held_messages > 0 {
            text.push_str(" · ");
            text.push_str(&m::worktree_setup_held(run.held_messages as usize));
        }
        let held = (text.len() > shown).then(|| {
            (
                shown..text.len(),
                HighlightStyle {
                    color: Some(colors.muted_foreground),
                    ..Default::default()
                },
            )
        });
        let title = StyledText::new(text).with_highlights(held);
        let summary = files_summary(&run);
        let output = (running || failed) && !run.output.is_empty();
        let thread_id = run.thread_id.clone();
        let engine = workspace.engine.clone();

        div().id("worktree-setup").child(
            Lane::new().child(
                div()
                    .mb_2()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .py_1p5()
                    .pl_3()
                    .pr_1p5()
                    .rounded_xl()
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.muted.opacity(0.4))
                    .text_xs()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(icon)
                            .child(div().flex_1().min_w_0().truncate().child(title))
                            .when_some(run.terminal_id.clone(), |this, terminal_id| {
                                let thread_id = thread_id.clone();
                                this.child(
                                    Button::new("worktree-setup-terminal")
                                        .label(m::worktree_setup_open_terminal())
                                        .size(ButtonSize::Xs)
                                        .variant(ButtonVariant::Ghost)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.show_terminal(&thread_id, &terminal_id, cx)
                                        })),
                                )
                            })
                            .when(failed, |this| {
                                let (engine, thread_id) = (engine.clone(), thread_id.clone());
                                this.child(
                                    Button::new("worktree-setup-again")
                                        .label(m::worktree_setup_run_again())
                                        .size(ButtonSize::Xs)
                                        .variant(ButtonVariant::Ghost)
                                        .on_click(move |_, _, cx| {
                                            let (engine, thread_id) =
                                                (engine.clone(), thread_id.clone());
                                            cx.background_spawn(async move {
                                                engine.rerun_worktree_setup(&thread_id).ok();
                                            })
                                            .detach();
                                        }),
                                )
                            })
                            .when(failed && run.held_messages > 0, |this| {
                                let (engine, thread_id) = (engine.clone(), thread_id.clone());
                                this.child(
                                    Button::new("worktree-setup-skip")
                                        .label(m::worktree_setup_send_anyway())
                                        .size(ButtonSize::Xs)
                                        .on_click(move |_, _, cx| {
                                            let (engine, thread_id) =
                                                (engine.clone(), thread_id.clone());
                                            cx.background_spawn(async move {
                                                engine.skip_worktree_setup(&thread_id).ok();
                                            })
                                            .detach();
                                        }),
                                )
                            })
                            .when(!running, |this| {
                                let thread_id = thread_id.clone();
                                this.child(
                                    Button::new("worktree-setup-dismiss")
                                        .icon(IconName::X)
                                        .size(ButtonSize::IconXs)
                                        .variant(ButtonVariant::Ghost)
                                        .tooltip(Tooltip::new(m::worktree_setup_dismiss()))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.dismiss(&thread_id, cx)
                                        })),
                                )
                            }),
                    )
                    .when(!summary.is_empty(), |this| {
                        this.child(
                            div()
                                .pr_1p5()
                                .truncate()
                                .text_color(colors.muted_foreground)
                                .child(summary),
                        )
                    })
                    .when(output, |this| {
                        this.child(
                            div()
                                .pr_1p5()
                                .overflow_hidden()
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_size(px(11.))
                                .line_height(px(16.))
                                .text_color(colors.muted_foreground)
                                .children(
                                    run.output
                                        .iter()
                                        .map(|line| div().truncate().child(line.clone())),
                                ),
                        )
                    }),
            ),
        )
    }
}
