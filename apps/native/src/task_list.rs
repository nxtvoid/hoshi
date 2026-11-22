//! The agent's task list, kept up to date in place as it works through it.

use crate::m;
use gpui_kit::component::Icon;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use hoshi_core::{Task, TaskStatus};
use hoshi_ui::LoopExt as _;
use hoshi_ui::assets::IconName;
use hoshi_ui::prelude::Colors;
use std::time::Duration;

/// How a task shows: its icon, and whether it is the one being worked on.
fn task_look(status: TaskStatus, working: bool) -> (IconName, bool) {
    match status {
        TaskStatus::Completed => (IconName::CircleCheck, false),
        TaskStatus::InProgress if working => (IconName::LoaderCircle, true),
        TaskStatus::InProgress | TaskStatus::Pending => (IconName::Circle, false),
    }
}

/// What a task reads as: how the agent words it while working on it, else its subject.
fn task_text(task: &Task, active: bool) -> &str {
    match &task.active_form {
        Some(form) if active && !form.is_empty() => form,
        _ => &task.subject,
    }
}

pub fn task_list_row(id: &str, tasks: &[Task], working: bool, colors: &Colors) -> AnyElement {
    let done = tasks
        .iter()
        .filter(|task| task.status == TaskStatus::Completed)
        .count();
    let rows = tasks.iter().map(|task| {
        let (icon, active) = task_look(task.status, working);
        let completed = task.status == TaskStatus::Completed;
        let icon = Icon::new(icon).size_3p5().flex_none().mt_0p5();
        let icon = if active {
            icon.text_color(colors.status_working)
                .with_loop(hoshi_ui::looping(Duration::from_secs(1)), |icon, delta| {
                    icon.transform(Transformation::rotate(percentage(delta)))
                })
                .into_any_element()
        } else {
            icon.into_any_element()
        };
        div()
            .flex()
            .items_start()
            .gap_2()
            .when(completed, |row| row.text_color(colors.muted_foreground))
            .child(icon)
            .child(
                div()
                    .min_w_0()
                    .when(completed, |text| text.line_through())
                    .when(active, |text| {
                        text.text_color(colors.foreground)
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .child(task_text(task, active).to_owned()),
            )
    });
    let selector = format!("task-list-{id}");
    div()
        .debug_selector(move || selector.clone())
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .flex()
                .min_h_6()
                .items_center()
                .gap_2()
                .text_sm()
                .text_color(colors.muted_foreground)
                .child(Icon::new(IconName::ListTodo).size_4().flex_none())
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(colors.foreground)
                        .child(m::task_list_progress(done, tasks.len())),
                ),
        )
        .child(
            div()
                .ml_2()
                .pl_4()
                .border_l_1()
                .border_color(colors.border)
                .flex()
                .flex_col()
                .gap_0p5()
                .text_sm()
                .text_color(colors.foreground)
                .children(rows),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{task_look, task_text};
    use hoshi_core::{Task, TaskStatus};
    use hoshi_ui::assets::IconName;

    fn task(status: TaskStatus) -> Task {
        Task {
            subject: "Write a.txt".into(),
            active_form: Some("Writing a.txt".into()),
            status,
        }
    }

    #[test]
    fn the_task_in_progress_spins_and_reads_as_its_active_form_only_while_the_turn_runs() {
        let running = task(TaskStatus::InProgress);
        assert_eq!(
            task_look(running.status, true),
            (IconName::LoaderCircle, true)
        );
        assert_eq!(task_text(&running, true), "Writing a.txt");
        assert_eq!(task_look(running.status, false), (IconName::Circle, false));
        assert_eq!(task_text(&running, false), "Write a.txt");
        assert_eq!(
            task_look(TaskStatus::Completed, true),
            (IconName::CircleCheck, false)
        );
    }
}
