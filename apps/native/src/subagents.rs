//! A subagent the agent started, as a card in the transcript: what it does or found, what it
//! runs on and how it stands, with a stop button while it runs. Clicking it opens the
//! subagent's own transcript.

use crate::composer::effort_label;
use crate::details::agents::{AgentState, subagent_state};
use crate::m;
use crate::transcript::TranscriptView;
use crate::turn_status::elapsed;
use chrono::{DateTime, Utc};
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::{Command, ItemContent, StreamKind, ThreadItem, ToolDetail};
use hoshi_ui::assets::IconName;
use hoshi_ui::prelude::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Asks to show a subagent in place of the thread, by the call that started it.
pub struct OpenSubagent(pub String);

impl EventEmitter<OpenSubagent> for TranscriptView {}

/// What the agent asked a subagent: its description, its kind and its prompt.
pub struct SubagentInput {
    pub description: String,
    pub kind: String,
    pub prompt: String,
}

pub fn subagent_input(input: &str) -> SubagentInput {
    let value: serde_json::Value = serde_json::from_str(input).unwrap_or_default();
    let text = |key: &str| {
        value
            .get(key)
            .and_then(|text| text.as_str())
            .unwrap_or_default()
            .to_owned()
    };
    SubagentInput {
        description: text("description"),
        kind: text("subagent_type"),
        prompt: text("prompt"),
    }
}

/// The first line of a text, or its last while it is still being written, without markdown
/// marks.
pub fn preview(text: &str, streaming: bool) -> String {
    let mut lines = text
        .lines()
        .map(crate::markdown::plain_text)
        .filter(|line| !line.is_empty());
    if streaming {
        lines.next_back()
    } else {
        lines.next()
    }
    .unwrap_or_default()
}

/// The model a subagent runs on, as the catalog names it, and its effort.
pub fn run_label(detail: Option<&ToolDetail>, cx: &App) -> Option<String> {
    let Some(ToolDetail::Subagent { model, effort }) = detail else {
        return None;
    };
    Some(model_label(model, *effort, cx))
}

/// A model as the catalog names it, with its effort.
pub fn model_label(model: &str, effort: Option<hoshi_core::Effort>, cx: &App) -> String {
    let name = crate::markdown::workspace(cx)
        .and_then(|workspace| {
            workspace.read(cx).models.iter().find_map(|catalog| {
                catalog
                    .models
                    .iter()
                    .find(|candidate| candidate.slug == model)
                    .map(|candidate| candidate.short_name.clone())
            })
        })
        .unwrap_or_else(|| model.to_owned());
    match effort {
        Some(effort) => format!("{name} · {}", effort_label(effort)),
        None => name,
    }
}

fn status_label(state: AgentState, started_at: &str, ended_at: Option<&str>) -> String {
    match state {
        AgentState::Running | AgentState::Waiting => {
            let started = DateTime::parse_from_rfc3339(started_at)
                .map(|time| time.with_timezone(&Utc).timestamp_millis())
                .unwrap_or_default();
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_millis() as i64);
            let running = elapsed(if started > 0 { now - started } else { 0 });
            if state == AgentState::Waiting {
                m::delegation_waiting_for(running)
            } else {
                m::delegation_running_for(running)
            }
        }
        AgentState::Stopped => match ended_at.and_then(clock_time) {
            Some(time) => m::delegation_stopped_at(time),
            None => m::agent_stopped(),
        },
        AgentState::Failed => match ended_at.and_then(clock_time) {
            Some(time) => m::delegation_failed_at(time),
            None => m::agent_failed(),
        },
        AgentState::Done => match ended_at.and_then(clock_time) {
            Some(time) => m::delegation_finished_at(time),
            None => m::agent_finished(),
        },
    }
}

/// The Claude mark of the agent, with a dot for how it stands when that is known.
pub fn avatar(state: Option<AgentState>, provider: &str, colors: &Colors) -> Div {
    let dot = state.map(|state| match state {
        AgentState::Running => colors.status_working,
        AgentState::Waiting => colors.status_waiting,
        AgentState::Failed => colors.status_error,
        AgentState::Stopped => colors.muted_foreground,
        AgentState::Done => colors.status_done,
    });
    div()
        .relative()
        .size_6()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(colors.border.opacity(0.7))
        .bg(colors.muted)
        .child(
            svg()
                .path(crate::composer::provider_mark(provider))
                .size_3p5()
                .text_color(crate::composer::provider_tint(provider, colors)),
        )
        .children(dot.map(|dot| {
            div()
                .absolute()
                .right(px(-1.))
                .bottom(px(-1.))
                .size_2()
                .rounded_full()
                .border_2()
                .border_color(colors.background)
                .bg(dot)
        }))
}

pub fn status(state: AgentState, started_at: &str, ended_at: Option<&str>, colors: &Colors) -> Div {
    let icon = match state {
        AgentState::Running => IconName::LoaderCircle,
        AgentState::Waiting => IconName::MessageCircleQuestionMark,
        AgentState::Stopped => IconName::CircleStop,
        AgentState::Failed => IconName::TriangleAlert,
        _ => IconName::CircleCheck,
    };
    let icon = Icon::new(icon).size_3p5().flex_none();
    let icon = match state {
        AgentState::Running => icon
            .with_loop(hoshi_ui::looping(Duration::from_secs(1)), |icon, delta| {
                icon.transform(Transformation::rotate(percentage(delta)))
            })
            .into_any_element(),
        AgentState::Failed => icon.text_color(colors.destructive).into_any_element(),
        AgentState::Waiting => icon.text_color(colors.status_waiting).into_any_element(),
        _ => icon.into_any_element(),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1()
        .text_xs()
        .text_color(colors.muted_foreground)
        .child(icon)
        .child(status_label(state, started_at, ended_at))
}

/// What a running subagent last wrote, from its streamed output.
fn progress(item: &ThreadItem, streaming: &[crate::state::Streaming]) -> Option<String> {
    streaming
        .iter()
        .filter(|stream| stream.item_id == item.id && stream.kind == StreamKind::ToolOutput)
        .map(|stream| preview(&stream.text, true))
        .rfind(|line| !line.is_empty())
}

/// A subagent's card, alone or `grouped` as a line of its group's card.
pub fn subagent_row(
    item: &ThreadItem,
    grouped: bool,
    streaming: &[crate::state::Streaming],
    engine: Option<std::sync::Arc<hoshi_engine::Engine>>,
    view: WeakEntity<TranscriptView>,
    colors: &Colors,
    cx: &App,
) -> AnyElement {
    let ItemContent::ToolCall {
        input,
        output,
        is_error,
        interrupted,
        detail,
        ..
    } = &item.content
    else {
        return div().into_any_element();
    };
    let state = subagent_state(output.is_some(), *is_error, *interrupted);
    let running = state == AgentState::Running;
    let asked = subagent_input(input);
    let title: SharedString = if asked.description.is_empty() {
        m::subagent().into()
    } else {
        asked.description.clone().into()
    };
    let detail_text = if running {
        progress(item, streaming)
    } else if !*interrupted {
        output
            .as_deref()
            .map(|output| preview(output, false))
            .filter(|line| !line.is_empty())
    } else {
        None
    };
    let failed = state == AgentState::Failed;
    let run = run_label(detail.as_ref(), cx);
    let id = item.id.clone();
    let thread_id = item.thread_id.clone();
    let open_id = item.id.clone();
    div()
        .id(SharedString::from(format!("subagent-{}", item.id)))
        .debug_selector({
            let selector = format!("subagent-{}", item.id);
            move || selector.clone()
        })
        .relative()
        .flex()
        .w_full()
        .items_center()
        .gap(px(10.))
        .text_sm()
        .cursor_pointer()
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
        .hover(|card| card.bg(colors.foreground.opacity(0.05)))
        .press_selects_nothing()
        .on_click(move |_, _, cx| {
            view.update(cx, |_, cx| cx.emit(OpenSubagent(open_id.clone())))
                .ok();
        })
        .child(avatar(
            Some(state),
            &thread_provider(&item.thread_id, cx),
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
                        .flex()
                        .min_w_0()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(colors.foreground)
                                .child(title),
                        )
                        .when(!asked.kind.is_empty(), |line| {
                            line.child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(colors.muted_foreground)
                                    .child(asked.kind.clone()),
                            )
                        }),
                )
                .children(detail_text.map(|detail| {
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(if failed {
                            colors.destructive
                        } else {
                            colors.muted_foreground
                        })
                        .child(detail)
                })),
        )
        .children(run.map(|run| {
            div()
                .flex_none()
                .text_xs()
                .text_color(colors.muted_foreground)
                .child(run)
        }))
        .child(status(state, &item.created_at, None, colors))
        .when(running, |card| {
            card.child(
                Button::new(SharedString::from(format!("stop-subagent-{id}")))
                    .label(m::background_task_stop())
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        let Some(engine) = engine.clone() else {
                            return;
                        };
                        let command = Command::SubagentStop {
                            thread_id: thread_id.clone(),
                            tool_use_id: id.clone(),
                        };
                        cx.background_spawn(async move { engine.dispatch(command) })
                            .detach();
                    }),
            )
        })
        .child(open_arrow(colors))
        .into_any_element()
}

/// The mark of a card that opens the agent's own view.
/// The provider the thread runs on; a subagent's transcript id names its thread first.
pub fn thread_provider(thread_id: &str, cx: &App) -> String {
    let thread_id = thread_id.split(':').next().unwrap_or(thread_id);
    crate::markdown::workspace(cx)
        .and_then(|workspace| {
            workspace
                .read(cx)
                .model
                .thread(thread_id)
                .map(|thread| thread.provider.clone())
        })
        .unwrap_or_else(hoshi_core::default_provider)
}

pub fn open_arrow(colors: &Colors) -> Icon {
    Icon::new(IconName::ArrowUpRight)
        .size_3p5()
        .flex_none()
        .text_color(colors.muted_foreground.opacity(0.7))
}

/// When an RFC 3339 time was, as the local clock read.
pub fn clock_time(at: &str) -> Option<String> {
    let time = DateTime::parse_from_rfc3339(at).ok()?;
    Some(
        time.with_timezone(&chrono::Local)
            .format("%H:%M")
            .to_string(),
    )
}

/// How many agents a group's header shows one by one; the rest are counted.
const MAX_AVATARS: usize = 5;

/// How a group's agents stand, as its header says it: "1 running · 2 done".
pub fn group_counts(states: &[Option<AgentState>]) -> Vec<String> {
    [
        (
            AgentState::Running,
            m::agents_running as fn(usize) -> String,
        ),
        (AgentState::Waiting, m::agents_waiting),
        (AgentState::Done, m::agents_done),
        (AgentState::Stopped, m::agents_stopped),
        (AgentState::Failed, m::agents_failed),
    ]
    .into_iter()
    .filter_map(|(state, label)| {
        let count = states.iter().filter(|s| **s == Some(state)).count();
        (count > 0).then(|| label(count))
    })
    .collect()
}

/// Agents started one after another, subagents and delegated tasks alike, as one card that
/// says how they stand; open, it has a line each. `agents` are their states and when each
/// started.
#[allow(clippy::too_many_arguments)]
pub fn agent_group(
    key: &str,
    provider: &str,
    agents: Vec<(Option<AgentState>, String)>,
    lines: Option<Vec<AnyElement>>,
    toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    colors: &Colors,
) -> AnyElement {
    let states: Vec<Option<AgentState>> = agents.iter().map(|(state, _)| *state).collect();
    let active: Vec<&String> = agents
        .iter()
        .filter(|(state, _)| state.is_some_and(AgentState::is_active))
        .map(|(_, started)| started)
        .collect();
    let started = active
        .iter()
        .filter_map(|at| DateTime::parse_from_rfc3339(at).ok())
        .map(|time| time.with_timezone(&Utc).timestamp_millis())
        .min();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64);
    let open = lines.is_some();
    let avatars = agents
        .iter()
        .take(MAX_AVATARS)
        .enumerate()
        .map(|(index, (state, _))| {
            avatar(*state, provider, colors)
                .when(index > 0, |avatar| avatar.ml(px(-4.)))
                .border_2()
                .border_color(colors.background)
        });
    let selector = format!("agent-group-{key}");
    let header = div()
        .id(SharedString::from(selector.clone()))
        .debug_selector(move || selector.clone())
        .flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap(px(10.))
        .px_3()
        .py_2()
        .rounded_xl()
        .when(open, |header| header.rounded_b_none())
        .text_sm()
        .cursor_pointer()
        .hover(|header| header.bg(colors.foreground.opacity(0.05)))
        .on_click(toggle)
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1p5()
                .child(div().flex().children(avatars))
                .when(agents.len() > MAX_AVATARS, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .child(m::agents_more(agents.len() - MAX_AVATARS)),
                    )
                }),
        )
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
                        .child(m::agents_count(agents.len())),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(if active.is_empty() {
                            colors.muted_foreground
                        } else {
                            colors.status_working
                        })
                        .child(group_counts(&states).join(" · ")),
                ),
        )
        .children(started.map(|started| {
            div()
                .flex_none()
                .text_xs()
                .text_color(colors.muted_foreground)
                .child(elapsed(now - started))
        }))
        .child(
            Icon::new(IconName::ChevronDown)
                .size_3p5()
                .flex_none()
                .text_color(colors.muted_foreground)
                .when(open, |icon| icon.rotate(percentage(0.5))),
        );
    div()
        .flex()
        .flex_col()
        .rounded_xl()
        .border_1()
        .border_color(colors.border.opacity(0.7))
        .child(header)
        .children(lines.map(|lines| {
            div()
                .flex()
                .flex_col()
                .p_1()
                .border_t_1()
                .border_color(colors.border.opacity(0.7))
                .children(lines)
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{preview, subagent_input};

    #[test]
    fn a_subagent_reads_from_its_call() {
        let asked = subagent_input(
            r#"{"description":"Wait 20s and say hola","subagent_type":"general","prompt":"Wait"}"#,
        );
        assert_eq!(asked.description, "Wait 20s and say hola");
        assert_eq!(asked.kind, "general");
        assert_eq!(
            preview("\n**Done.** All good\nmore", false),
            "Done. All good"
        );
        assert_eq!(preview("first\nlast line", true), "last line");
    }
}
