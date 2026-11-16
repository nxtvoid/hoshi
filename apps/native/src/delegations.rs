//! Tasks the agent delegated to child threads: a card each in the transcript with how the
//! child is doing and what it said last, the tray of a task that waits for the user's changes
//! to its plan, and the tasks still running once the agent rests. Each opens its child.

use crate::details::agents::{AgentState, busy_threads, task_state};
use crate::m;
use crate::state::Workspace;
use crate::subagents::{avatar, model_label, open_arrow, status};
use crate::task_runs::TaskOutcome;
use crate::turn_status::elapsed;
use chrono::{DateTime, Utc};
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::{ItemContent, Thread, ThreadItem, ThreadStatus};
use hoshi_ui::assets::IconName;
use hoshi_ui::prelude::*;
use serde_json::Value;
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

/// What a delegation call says: the task's title, its child thread once started, and why it
/// failed to start.
#[derive(Debug, PartialEq, Eq)]
pub struct Delegation {
    pub title: String,
    pub task_id: Option<String>,
    pub error: Option<String>,
}

pub fn delegation_of(input: &str, output: Option<&str>, is_error: bool) -> Delegation {
    let parse = |text: &str| serde_json::from_str::<Value>(text).ok();
    let input = parse(input);
    let answer = output.filter(|_| !is_error).and_then(parse);
    let text = |value: &Option<Value>, key: &str| {
        value
            .as_ref()
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    Delegation {
        title: text(&answer, "title")
            .or_else(|| text(&input, "title"))
            .or_else(|| text(&input, "task"))
            .unwrap_or_default(),
        task_id: text(&answer, "taskId"),
        error: if is_error {
            output.map(str::to_owned)
        } else {
            None
        },
    }
}

/// A preview as plain text: links keep their label, markdown marks go.
pub fn plain_preview(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let link = after.find("](").and_then(|close| {
            let label = &after[..close];
            let target = &after[close + 2..];
            (!label.is_empty() && !label.contains(']'))
                .then(|| target.find(')').map(|end| (label, &target[end + 1..])))
                .flatten()
        });
        match link {
            Some((label, tail)) => {
                out.push_str(label);
                rest = tail;
            }
            None => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    let out = out.replace("**", "").replace("__", "").replace('`', "");
    let out = out.trim_start_matches(|c: char| c.is_whitespace() || "#>*-".contains(c));
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A task that waits for the user's changes to its plan.
pub fn planning_with_user(thread: &Thread, pending: &HashSet<String>) -> bool {
    thread.parent_thread_id.is_some()
        && thread.plan_mode
        && thread.status == ThreadStatus::Idle
        && pending.contains(&thread.id)
}

/// How the workspace's tasks stand, read once per frame for every card that needs it.
pub struct TaskBoard<'a> {
    workspace: &'a Workspace,
    busy: HashSet<String>,
}

impl<'a> TaskBoard<'a> {
    pub fn new(workspace: &'a Workspace) -> Self {
        let threads: Vec<&Thread> = workspace.model.threads().collect();
        let busy = busy_threads(
            &threads,
            &workspace.pending_tasks,
            workspace.background_tasks.keys(),
        );
        Self { workspace, busy }
    }

    pub fn thread(&self, id: &str) -> Option<&'a Thread> {
        self.workspace.model.thread(id)
    }

    pub fn state(&self, thread: &Thread) -> AgentState {
        task_state(thread, &self.workspace.pending_tasks, &self.busy)
    }

    fn planning(&self, thread: &Thread) -> bool {
        planning_with_user(thread, &self.workspace.pending_tasks)
    }

    fn preview(&self, id: &str) -> Option<String> {
        self.workspace
            .model
            .task_previews()
            .into_iter()
            .find(|preview| preview.thread_id == id)
            .map(|preview| plain_preview(&preview.text))
    }

    /// The tasks `thread_id` delegated that are in flight and run, oldest first.
    pub fn running(&self, thread_id: &str) -> Vec<&'a Thread> {
        let mut running: Vec<&Thread> = self
            .workspace
            .model
            .threads()
            .filter(|thread| thread.parent_thread_id.as_deref() == Some(thread_id))
            .filter(|thread| !thread.task_cancelled)
            .filter(|thread| {
                self.workspace.pending_tasks.contains(&thread.id) || self.busy.contains(&thread.id)
            })
            .filter(|thread| self.state(thread) == AgentState::Running)
            .collect();
        running.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        running
    }
}

/// How the delegated task of `item` stands, for a group's header: unknown before it starts.
pub fn delegation_state(item: &ThreadItem, board: Option<&TaskBoard>) -> Option<AgentState> {
    let ItemContent::ToolCall {
        input,
        output,
        is_error,
        ..
    } = &item.content
    else {
        return None;
    };
    let delegation = delegation_of(input, output.as_deref(), *is_error);
    let child = board.zip(delegation.task_id.as_deref());
    match child.and_then(|(board, id)| board.thread(id).map(|thread| board.state(thread))) {
        Some(state) => Some(state),
        None if delegation.error.is_some() => Some(AgentState::Failed),
        None => None,
    }
}

/// When the delegated task of `item` started: its thread's start, else the call's.
pub fn delegation_started<'a>(item: &'a ThreadItem, board: Option<&'a TaskBoard>) -> &'a str {
    let ItemContent::ToolCall { output, .. } = &item.content else {
        return &item.created_at;
    };
    let task = output
        .as_deref()
        .and_then(|output| serde_json::from_str::<Value>(output).ok())
        .and_then(|value| value.get("taskId")?.as_str().map(str::to_owned));
    board
        .zip(task)
        .and_then(|(board, id)| board.thread(&id))
        .map_or(&item.created_at, |thread| &thread.created_at)
}

pub fn open_thread(thread_id: String) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    move |_, _, cx| {
        cx.stop_propagation();
        if let Some(workspace) = crate::markdown::workspace(cx) {
            workspace.update(cx, |workspace, cx| workspace.open_thread(&thread_id, cx));
        }
    }
}

/// Hands a task's answer back to its thread now, as it stands.
/// Above a task's transcript: the thread that delegated it, and the way back to it.
pub fn delegated_by_line(workspace: &Entity<Workspace>, cx: &App) -> Option<AnyElement> {
    let workspace = workspace.read(cx);
    let task = workspace
        .open_thread_id(cx)
        .and_then(|id| workspace.model.thread(&id))?;
    let parent = workspace.model.thread(task.parent_thread_id.as_deref()?)?;
    let colors = cx.hoshi();
    Some(
        div()
            .bg(colors.background)
            .child(
                Lane::new().child(
                    div()
                        .debug_selector(|| "delegated-by".into())
                        .pt_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(Icon::new(IconName::GitFork).size_3p5().flex_none())
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .child(m::delegated_by(&parent.title)),
                        )
                        .child(
                            Button::new("delegated-by-back")
                                .icon(IconName::ArrowLeft)
                                .label(m::delegated_back())
                                .size(ButtonSize::Xs)
                                .variant(ButtonVariant::Ghost)
                                .on_click(open_thread(parent.id.clone())),
                        ),
                ),
            )
            .into_any_element(),
    )
}

pub fn send_task_answer(thread_id: String, cx: &mut App) {
    let Some(workspace) = crate::markdown::workspace(cx) else {
        return;
    };
    let engine = workspace.read(cx).engine.clone();
    cx.background_spawn(async move { engine.send_task_answer(&thread_id) })
        .detach();
}

/// The planning trays drawn this frame inside the viewport, by task.
#[derive(Default)]
struct ShownTrays(HashSet<String>);

impl Global for ShownTrays {}

/// After the transcript's rows: tells the workspace which planning trays are on screen, so
/// the composer needn't point at them.
pub fn report_shown_trays() -> impl IntoElement {
    canvas(
        |_, _, _| {},
        |_, _, _, cx| {
            let shown = std::mem::take(&mut cx.default_global::<ShownTrays>().0);
            let Some(workspace) = crate::markdown::workspace(cx) else {
                return;
            };
            if workspace.read(cx).tasks_on_screen != shown {
                cx.defer(move |cx| {
                    workspace.update(cx, |workspace, cx| workspace.set_tasks_on_screen(shown, cx))
                });
            }
        },
    )
    .absolute()
    .size_0()
}

fn cancel_task(thread_id: String, cx: &mut App) {
    let Some(workspace) = crate::markdown::workspace(cx) else {
        return;
    };
    let engine = workspace.read(cx).engine.clone();
    cx.background_spawn(async move { engine.cancel_task(&thread_id) })
        .detach();
}

/// A delegated task's card, alone or `grouped` as a line of its group's card.
pub fn delegation_row(item: &ThreadItem, grouped: bool, colors: &Colors, cx: &App) -> AnyElement {
    let ItemContent::ToolCall {
        input,
        output,
        is_error,
        ..
    } = &item.content
    else {
        return div().into_any_element();
    };
    let delegation = delegation_of(input, output.as_deref(), *is_error);
    let workspace = crate::markdown::workspace(cx);
    let workspace = workspace.as_ref().map(|workspace| workspace.read(cx));
    let board = workspace.map(TaskBoard::new);
    let child = board
        .as_ref()
        .zip(delegation.task_id.as_deref())
        .and_then(|(board, id)| board.thread(id));
    let state = match (child, &board) {
        (Some(child), Some(board)) => Some(board.state(child)),
        _ if delegation.error.is_some() => Some(AgentState::Failed),
        _ => None,
    };
    let planning = child
        .zip(board.as_ref())
        .is_some_and(|(child, board)| board.planning(child));
    let title: SharedString = child
        .map(|child| child.title.clone())
        .unwrap_or(delegation.title)
        .into();
    let preview = child
        .zip(board.as_ref())
        .and_then(|(child, board)| board.preview(&child.id))
        .filter(|preview| !preview.is_empty());
    let failed = delegation.error.is_some();
    let detail: SharedString = delegation
        .error
        .or(preview)
        .unwrap_or_else(m::delegation_task)
        .into();
    let run = child.and_then(|child| {
        let model = workspace
            .and_then(|workspace| workspace.chosen_model(&child.provider, child.model.as_deref()))
            .map(|model| model.slug.as_str())
            .or(child.model.as_deref())?;
        Some(model_label(model, child.effort, cx))
    });
    let running = state == Some(AgentState::Running);
    let selector = format!("delegation-{}", item.id);
    let line = div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .relative()
        .flex()
        .w_full()
        .items_center()
        .gap(px(10.))
        .text_sm()
        .map(|card| {
            if grouped {
                card.rounded_lg().px_2().py_1p5()
            } else {
                card.rounded_xl()
                    .border_1()
                    .border_color(colors.border.opacity(0.7))
                    .px_3()
                    .py_2()
            }
        })
        .when(planning, |card| card.bg(colors.background))
        .when_some(child, |card, child| {
            card.cursor_pointer()
                .hover(|card| card.bg(colors.foreground.opacity(0.05)))
                .on_click(open_thread(child.id.clone()))
        })
        .child(avatar(
            state,
            &child.map_or_else(
                || crate::subagents::thread_provider(&item.thread_id, cx),
                |child| child.provider.clone(),
            ),
            colors,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(colors.foreground)
                        .child(title),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(if failed {
                            colors.destructive
                        } else {
                            colors.muted_foreground
                        })
                        .child(detail),
                ),
        )
        .child(
            div()
                .flex_none()
                .px_1p5()
                .rounded_full()
                .border_1()
                .border_color(colors.border.opacity(0.7))
                .text_xs()
                .text_color(colors.muted_foreground)
                .child(m::agent_own_thread()),
        )
        .children(run.map(|run| {
            div()
                .flex_none()
                .text_xs()
                .text_color(colors.muted_foreground)
                .child(run)
        }))
        .children(state.filter(|_| !planning).map(|state| {
            status(
                state,
                child.map_or(item.created_at.as_str(), |child| &child.created_at),
                child.map(|child| child.updated_at.as_str()),
                &item.id,
                colors,
            )
        }))
        .when_some(child.filter(|_| running), |card, child| {
            let id = child.id.clone();
            card.child(
                Button::new(SharedString::from(format!("stop-task-{}", child.id)))
                    .label(m::background_task_stop())
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        cancel_task(id.clone(), cx);
                    }),
            )
        })
        .when(child.is_some(), |card| card.child(open_arrow(colors)));
    match child.filter(|_| planning) {
        Some(child) => div()
            .flex()
            .flex_col()
            .child(line)
            .child(planning_tray(child, colors))
            .into_any_element(),
        None => line.into_any_element(),
    }
}

/// Under a task's card, while it waits for the user's changes to its plan: what it waits for
/// and the ways to answer.
fn planning_tray(child: &Thread, colors: &Colors) -> AnyElement {
    let waiting = colors.status_waiting;
    let (cancel_id, send_id, title) = (child.id.clone(), child.id.clone(), child.title.clone());
    let shown_id = child.id.clone();
    let action = |id: &str, icon: IconName, label: String| {
        Button::new(SharedString::from(format!("{id}-{}", child.id)))
            .icon(icon)
            .size(ButtonSize::Xs)
            .variant(ButtonVariant::Ghost)
            .tooltip(label)
    };
    div()
        .relative()
        .mx_3p5()
        .mt(px(-8.))
        .flex()
        .items_center()
        .gap_1()
        .pt_3()
        .pr_1p5()
        .pb_1()
        .pl_3()
        .rounded_b_lg()
        .border_1()
        .border_t_0()
        .border_color(waiting.opacity(0.2))
        .bg(waiting.opacity(0.05))
        .text_xs()
        .child(
            canvas(
                move |bounds, window, cx| {
                    if window.content_mask().bounds.intersects(&bounds) {
                        cx.default_global::<ShownTrays>().0.insert(shown_id.clone());
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full(),
        )
        .child(
            Icon::new(IconName::MessageCircleQuestionMark)
                .size_3p5()
                .flex_none()
                .text_color(waiting),
        )
        .child(
            div()
                .ml_1()
                .min_w_0()
                .flex_1()
                .truncate()
                .text_color(waiting.opacity(0.9))
                .child(m::delegation_waiting_changes()),
        )
        .child(
            action("cancel-task", IconName::CircleX, m::delegation_cancel())
                .text_color(colors.destructive)
                .on_click(move |_, window, cx| {
                    let request = ConfirmRequest::new(
                        m::delegation_cancel_title(&title),
                        m::delegation_cancel(),
                    )
                    .description(m::delegation_cancel_description())
                    .cancel_label(m::delegation_cancel_keep())
                    .destructive()
                    .icon(IconName::CircleX);
                    let confirmed = DialogLayer::confirm(request, window, cx);
                    let id = cancel_id.clone();
                    window
                        .spawn(cx, async move |cx| {
                            if confirmed.await {
                                cx.update(|_, cx| cancel_task(id, cx)).ok();
                            }
                        })
                        .detach();
                }),
        )
        .child(
            action("send-plan", IconName::Send, m::delegation_send_plan())
                .text_color(colors.send)
                .on_click(move |_, _, cx| send_task_answer(send_id.clone(), cx)),
        )
        .child(
            action("change-plan", IconName::Pencil, m::delegation_change_plan())
                .variant(ButtonVariant::Primary)
                .on_click(open_thread(child.id.clone())),
        )
        .into_any_element()
}

/// The agent's reply to a task's answer, while the work on its tasks goes on.
pub fn task_note(
    id: &str,
    text: impl IntoElement,
    outcome: TaskOutcome,
    colors: &Colors,
) -> AnyElement {
    let (icon, tone) = match outcome {
        TaskOutcome::Completed => (IconName::Check, colors.status_done),
        TaskOutcome::Failed => (IconName::TriangleAlert, colors.destructive),
        TaskOutcome::Stopped => (IconName::CircleX, colors.muted_foreground),
    };
    let selector = format!("task-note-{id}");
    div()
        .debug_selector(move || selector)
        .flex()
        .items_start()
        .gap_2()
        .child(
            Icon::new(icon)
                .size_3p5()
                .flex_none()
                .mt_1()
                .text_color(tone),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .text_sm()
                .text_color(colors.muted_foreground)
                .child(text),
        )
        .into_any_element()
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

/// At the end of the transcript while its agent rests: the tasks it delegated that still run,
/// whose answers it waits for.
pub fn task_waits(thread_id: &str, colors: &Colors, cx: &App) -> AnyElement {
    let Some(workspace) = crate::markdown::workspace(cx) else {
        return div().into_any_element();
    };
    let workspace = workspace.read(cx);
    let board = TaskBoard::new(workspace);
    let now = now_millis();
    let lines = board.running(thread_id).into_iter().map(|child| {
        let started = DateTime::parse_from_rfc3339(&child.created_at)
            .map(|time| time.with_timezone(&Utc).timestamp_millis())
            .unwrap_or(now);
        let title: SharedString = if child.title.is_empty() {
            m::delegation_task().into()
        } else {
            child.title.clone().into()
        };
        let id = child.id.clone();
        div()
            .flex()
            .min_w_0()
            .items_center()
            .gap_2()
            .text_sm()
            .text_color(colors.muted_foreground)
            .child(Icon::new(IconName::GitFork).size_4().flex_none())
            .child(div().flex_none().child(m::background_waiting()))
            .child(div().flex_none().child("·"))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(colors.foreground.opacity(0.8))
                    .child(title),
            )
            .child(div().flex_none().child("·"))
            .child(div().flex_none().child(elapsed(now - started)))
            .child(
                Button::new(SharedString::from(format!("open-task-{}", child.id)))
                    .label(m::delegation_open())
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Ghost)
                    .on_click(open_thread(child.id.clone())),
            )
            .child(
                Button::new(SharedString::from(format!("stop-task-wait-{}", child.id)))
                    .label(m::background_task_stop())
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, _, cx| cancel_task(id.clone(), cx)),
            )
    });
    div()
        .flex()
        .flex_col()
        .gap_2()
        .children(lines)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{Delegation, delegation_of, plain_preview};

    #[test]
    fn a_delegation_reads_its_title_and_task_from_the_answer_before_the_input() {
        assert_eq!(
            delegation_of(
                r#"{"task":"Fix the build"}"#,
                Some(r#"{"taskId":"t2","title":"Fix CI"}"#),
                false
            ),
            Delegation {
                title: "Fix CI".into(),
                task_id: Some("t2".into()),
                error: None,
            }
        );
        assert_eq!(
            delegation_of(r#"{"task":"Fix the build"}"#, None, false).title,
            "Fix the build"
        );
        assert_eq!(
            delegation_of(r#"{"title":"Docs"}"#, Some("no such model"), true),
            Delegation {
                title: "Docs".into(),
                task_id: None,
                error: Some("no such model".into()),
            }
        );
    }

    #[test]
    fn a_preview_keeps_link_labels_and_drops_marks() {
        assert_eq!(
            plain_preview("## **Done**: see [the PR](https://x/1) and `a.rs`\n  next"),
            "Done: see the PR and a.rs next"
        );
        assert_eq!(plain_preview("[not a link] here"), "[not a link] here");
    }
}
