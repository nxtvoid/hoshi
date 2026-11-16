//! The tasks the thread delegated that wait for the user's changes to their plan, and whose
//! card is off screen, in one line above the composer: the oldest, and how many more.

use crate::delegations::{open_thread, planning_with_user, send_task_answer};
use crate::m;
use crate::state::Workspace;
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::Thread;
use hoshi_ui::assets::IconName;
use hoshi_ui::prelude::*;

/// The tasks of `thread_id` planning with the user whose card is off screen, oldest first.
pub fn planning_off_screen<'a>(workspace: &'a Workspace, thread_id: &str) -> Vec<&'a Thread> {
    let mut planning: Vec<&Thread> = workspace
        .model
        .threads()
        .filter(|thread| thread.parent_thread_id.as_deref() == Some(thread_id))
        .filter(|thread| planning_with_user(thread, &workspace.pending_tasks))
        .filter(|thread| !workspace.tasks_on_screen.contains(&thread.id))
        .collect();
    planning.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    planning
}

pub fn planning_note(workspace: &Entity<Workspace>, cx: &App) -> Option<AnyElement> {
    let workspace = workspace.read(cx);
    let thread_id = workspace.open_thread_id(cx)?;
    let planning = planning_off_screen(workspace, &thread_id);
    let task = planning.first()?;
    let colors = cx.hoshi();
    let mut text = m::planning_task_note(&task.title);
    if planning.len() > 1 {
        text.push(' ');
        text.push_str(&m::planning_tasks_more(planning.len() - 1));
    }
    let send = task.id.clone();
    Some(
        div()
            .debug_selector(|| "planning-note".into())
            .flex()
            .items_center()
            .gap_2()
            .px_1()
            .pb_1()
            .text_xs()
            .text_color(colors.muted_foreground)
            .child(Icon::new(IconName::GitFork).size_3p5().flex_none())
            .child(div().min_w_0().flex_1().truncate().child(text))
            .child(
                Button::new("planning-note-send")
                    .label(m::delegation_send_plan())
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Ghost)
                    .text_color(colors.muted_foreground)
                    .on_click(move |_, _, cx| send_task_answer(send.clone(), cx)),
            )
            .child(
                Button::new("planning-note-open")
                    .label(m::delegation_open())
                    .suffix(Icon::new(IconName::ChevronRight).size_3p5())
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Ghost)
                    .on_click(open_thread(task.id.clone())),
            )
            .into_any_element(),
    )
}
