//! The agent's work in the transcript: tool calls, reasoning, and runs of them as one row.

use crate::request_rows::{CallApproval, approval_badge};
use crate::state::ThreadState;
use crate::tool_calls::{
    CallStatus, HoshiTool, ToolAction, call_status, hoshi_label, input_lines, is_workflow,
    reasoning_preview, summarize, summary_text, thought_line, tool_action, tool_name, tool_target,
    verb,
};
use crate::transcript::TranscriptView;
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::*;
use hoshi_core::{Command, ItemContent, ThreadItem, ToolDetail};
use hoshi_ui::assets::brand;
use hoshi_ui::prelude::*;
use std::collections::{HashMap, HashSet};

const OUTPUT_MAX_LINES: usize = 400;
const OUTPUT_MAX_CHARS: usize = 40_000;
const LIVE_OUTPUT_LINES: usize = 40;

/// What a row needs to draw the work it holds.
pub struct WorkContext<'a> {
    pub thread: &'a ThreadState,
    pub approvals: &'a HashMap<String, CallApproval>,
    pub open: &'a HashSet<String>,
    pub view: &'a WeakEntity<TranscriptView>,
    pub colors: &'a Colors,
    /// Each turn's changes by the answer that shows them, as indices in the thread's items.
    pub turn_changes: &'a HashMap<String, usize>,
    /// The thread's agent is at work.
    pub working: bool,
    /// The row is history a fork inherited: shown without actions.
    pub read_only: bool,
    /// The answers a fork can start from, by item id.
    pub forkable: &'a HashSet<String>,
    /// The task list the running turn works through, by item id.
    pub live_tasks: Option<&'a str>,
}

impl WorkContext<'_> {
    pub fn toggle(&self, id: &str) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        let (view, id) = (self.view.clone(), id.to_owned());
        move |_, _, cx| {
            view.update(cx, |view, cx| view.toggle(&id, cx)).ok();
        }
    }
}

/// The key a run of work opens under: its first item's.
fn group_key(first_item: &str) -> String {
    format!("work:{first_item}")
}

fn action_icon(action: ToolAction, colors: &Colors) -> AnyElement {
    let name = match action {
        ToolAction::Command => IconName::SquareTerminal,
        ToolAction::Edit => IconName::FilePen,
        ToolAction::Read => IconName::File,
        ToolAction::Search => IconName::Search,
        ToolAction::Web => IconName::Globe,
        ToolAction::Hoshi(HoshiTool::Browser) => IconName::AppWindow,
        ToolAction::Hoshi(
            HoshiTool::OpenPullRequest
            | HoshiTool::WatchPullRequest
            | HoshiTool::UnwatchPullRequest,
        ) => IconName::GitPullRequest,
        ToolAction::Hoshi(_) => {
            return svg()
                .path(brand::MARK)
                .size_3p5()
                .mx(px(1.))
                .flex_none()
                .text_color(colors.muted_foreground)
                .into_any_element();
        }
        ToolAction::Other => IconName::Wrench,
    };
    icon(name, None).into_any_element()
}

fn icon(name: IconName, color: Option<Hsla>) -> impl IntoElement {
    let icon = Icon::new(name).size_4().flex_none();
    match color {
        Some(color) => icon.text_color(color),
        None => icon,
    }
}

fn label(text: impl Into<SharedString>, running: bool, colors: &Colors) -> AnyElement {
    let text = text.into();
    if running {
        Shimmer::new(text)
            .flex_none()
            .font_weight(FontWeight::MEDIUM)
            .text_color(colors.foreground)
            .into_any_element()
    } else {
        div()
            .flex_none()
            .font_weight(FontWeight::MEDIUM)
            .text_color(colors.foreground)
            .child(text)
            .into_any_element()
    }
}

fn target(text: String, cx: &App) -> impl IntoElement {
    div()
        .min_w_0()
        .flex_shrink(1.)
        .truncate()
        .font_family(cx.theme().mono_font_family.clone())
        .text_xs()
        .child(text.replace(['\n', '\r'], " "))
}

/// A run of tool calls and reasoning: one line naming what it did, or while it runs, what
/// runs now; it opens to each of them.
pub fn work_group(entries: &[&ThreadItem], live: bool, ctx: &WorkContext, cx: &App) -> AnyElement {
    let [first, ..] = entries else {
        return div().into_any_element();
    };
    if entries.len() == 1 && !live {
        return entry(first, ctx, cx);
    }
    let key = group_key(&first.id);
    let open = ctx.open.contains(&key);
    let colors = ctx.colors;
    let active = entries
        .iter()
        .rev()
        .copied()
        .find(|item| active_entry(item));
    let thought = live
        .then(|| {
            entries.iter().rev().find_map(|item| match &item.content {
                ItemContent::Reasoning { text } => Some(thought_line(text)),
                _ => None,
            })
        })
        .flatten()
        .filter(|line| !line.is_empty() && !open);
    let header = if live {
        let latest = active.unwrap_or(entries[entries.len() - 1]);
        let failed = matches!(
            &latest.content,
            ItemContent::ToolCall {
                is_error: true,
                interrupted: false,
                ..
            }
        );
        let line = live_line(latest, ctx, cx);
        if failed { line } else { vec![shimmer(line)] }
    } else {
        summary_line(entries, colors)
    };
    div()
        .flex()
        .flex_col()
        .when_some(thought, |group, thought| {
            group.child(
                div()
                    .flex()
                    .gap_2()
                    .py_0p5()
                    .text_sm()
                    .text_color(colors.muted_foreground)
                    .child(icon(IconName::Brain, None))
                    .child(div().min_w_0().line_clamp(4).child(thought)),
            )
        })
        .child(
            Disclosure::new(SharedString::from(key.clone()))
                .open(open)
                .nested(true)
                .on_toggle(ctx.toggle(&key))
                .children(header)
                .when(open, |run| {
                    run.body(
                        div()
                            .flex()
                            .flex_col()
                            .children(entries.iter().map(|item| entry(item, ctx, cx))),
                    )
                }),
        )
        .into_any_element()
}

/// What runs now, brightening and fading as it goes.
fn shimmer(line: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .min_w_0()
        .items_center()
        .gap_2()
        .children(line)
        .with_loop(
            hoshi_ui::looping(std::time::Duration::from_millis(1600))
                .with_easing(bounce(ease_in_out)),
            |line, delta| line.opacity(0.55 + 0.45 * delta),
        )
        .into_any_element()
}

/// A call still running.
pub fn active_entry(item: &ThreadItem) -> bool {
    matches!(
        &item.content,
        ItemContent::ToolCall {
            output: None,
            interrupted: false,
            ..
        }
    )
}

fn summary_line(entries: &[&ThreadItem], colors: &Colors) -> Vec<AnyElement> {
    let calls: Vec<(&str, &str)> = entries
        .iter()
        .filter_map(|item| match &item.content {
            ItemContent::ToolCall { name, input, .. } => Some((name.as_str(), input.as_str())),
            _ => None,
        })
        .collect();
    let failed = entries.iter().rev().find_map(|item| match &item.content {
        ItemContent::ToolCall {
            is_error,
            interrupted,
            ..
        } => Some(*is_error && !interrupted),
        _ => None,
    }) == Some(true);
    let actions: HashSet<ToolAction> = calls.iter().map(|(name, _)| tool_action(name)).collect();
    let mark = if failed {
        icon(IconName::CircleX, Some(colors.destructive)).into_any_element()
    } else if calls.is_empty() {
        icon(IconName::Brain, None).into_any_element()
    } else if actions.len() == 1 {
        action_icon(*actions.iter().next().unwrap_or(&ToolAction::Other), colors)
    } else {
        icon(IconName::Wrench, None).into_any_element()
    };
    let text = if calls.is_empty() {
        crate::m::reasoning_thought()
    } else {
        summary_text(&summarize(calls))
    };
    vec![
        mark,
        div().min_w_0().truncate().child(text).into_any_element(),
    ]
}

fn live_line(item: &ThreadItem, ctx: &WorkContext, cx: &App) -> Vec<AnyElement> {
    let colors = ctx.colors;
    match &item.content {
        ItemContent::ToolCall {
            name,
            input,
            output,
            is_error,
            interrupted,
            ..
        } => {
            let status = call_status(output.is_some(), *is_error, *interrupted);
            let action = tool_action(name);
            let running = status == CallStatus::Running;
            let shown = verb(action, running)
                .or_else(|| hoshi_label(name, running))
                .unwrap_or_else(|| tool_name(name).label);
            let mark = match status {
                CallStatus::Failed => {
                    icon(IconName::CircleX, Some(colors.destructive)).into_any_element()
                }
                _ => action_icon(action, colors),
            };
            let target_text = if matches!(action, ToolAction::Hoshi(_)) {
                String::new()
            } else {
                tool_target(name, input, ctx.thread.cwd.as_deref())
            };
            vec![
                mark,
                label(shown, status != CallStatus::Failed, colors),
                target(target_text, cx).into_any_element(),
            ]
        }
        _ => vec![
            icon(IconName::Brain, None).into_any_element(),
            label(crate::m::reasoning_thought(), false, colors),
        ],
    }
}

/// One entry of a run: a tool call or a thought.
pub fn entry(item: &ThreadItem, ctx: &WorkContext, cx: &App) -> AnyElement {
    match &item.content {
        ItemContent::Reasoning { text } => reasoning(&item.id, text, false, ctx),
        ItemContent::ToolCall { .. } => tool_call(item, ctx, cx),
        _ => div().into_any_element(),
    }
}

/// The model's reasoning, one line until opened.
pub fn reasoning(id: &str, text: &str, streaming: bool, ctx: &WorkContext) -> AnyElement {
    let open = ctx.open.contains(id);
    let colors = ctx.colors;
    Disclosure::new(SharedString::from(id.to_owned()))
        .open(open)
        .on_toggle(ctx.toggle(id))
        .child(icon(IconName::Brain, None))
        .when(streaming, |row| {
            row.child(label(crate::m::reasoning_thinking(), true, colors))
        })
        .child(
            div()
                .min_w_0()
                .truncate()
                .child(reasoning_preview(text, streaming)),
        )
        .when(open, |row| {
            row.body(
                div()
                    .id(SharedString::from(format!("{id}:body")))
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(colors.muted_foreground)
                    .child(crate::markdown::markdown(
                        SharedString::from(id.to_owned()),
                        text.to_owned(),
                        colors,
                    )),
            )
        })
        .into_any_element()
}

/// A tool call: what it did and on what, its status, and once opened its input and output,
/// or the diff of the file it changed.
pub fn tool_call(item: &ThreadItem, ctx: &WorkContext, cx: &App) -> AnyElement {
    let ItemContent::ToolCall {
        name,
        input,
        output,
        is_error,
        detail,
        interrupted,
        ..
    } = &item.content
    else {
        return div().into_any_element();
    };
    let colors = ctx.colors;
    let open = ctx.open.contains(&item.id);
    let status = call_status(output.is_some(), *is_error, *interrupted);
    let running = status == CallStatus::Running;
    let action = tool_action(name);
    let shown = tool_name(name);
    let target_text = tool_target(name, input, ctx.thread.cwd.as_deref());
    let change = match detail {
        Some(ToolDetail::FileChange {
            additions,
            deletions,
            ..
        }) => Some((*additions, *deletions)),
        _ => None,
    };
    let raw_input = (target_text.is_empty() && !is_empty_input(input)).then(|| input_lines(input));
    let has_output = output
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty());
    let expandable =
        !target_text.is_empty() || raw_input.is_some() || has_output || change.is_some() || running;
    let mark = match status {
        CallStatus::Stopped => icon(IconName::CircleStop, None).into_any_element(),
        CallStatus::Failed => icon(IconName::CircleX, Some(colors.destructive)).into_any_element(),
        _ => action_icon(action, colors),
    };
    let own = hoshi_label(name, running);
    let hoshi = own.is_some();
    let words = verb(action, running)
        .or(own)
        .unwrap_or_else(|| shown.label.clone());
    let body = (open && expandable).then(|| call_body(item, &target_text, raw_input, ctx));
    Disclosure::new(SharedString::from(item.id.clone()))
        .open(open)
        .expandable(expandable)
        .on_toggle(ctx.toggle(&item.id))
        .child(mark)
        .child(label(words, running, colors))
        .when_some(shown.source.filter(|_| !hoshi), |row, source| {
            row.child(div().flex_none().text_xs().child(source))
        })
        .when(!hoshi, |row| row.child(target(target_text.clone(), cx)))
        .child(div().flex_1())
        .when(running && is_workflow(name), |row| {
            row.child(stop_workflow(item, ctx))
        })
        .when_some(change, |row, (additions, deletions)| {
            row.child(DiffStat::new(additions, deletions))
        })
        .when_some(ctx.approvals.get(&item.id), |row, approval| {
            row.child(approval_badge(&item.id, approval, colors))
        })
        .when_some(body, |row, body| row.body(body))
        .into_any_element()
}

fn stop_workflow(item: &ThreadItem, ctx: &WorkContext) -> Button {
    let engine = ctx.thread.engine.clone();
    let command = Command::SubagentStop {
        thread_id: item.thread_id.clone(),
        tool_use_id: item.id.clone(),
    };
    Button::new(SharedString::from(format!("stop-workflow-{}", item.id)))
        .label(crate::m::background_task_stop())
        .size(ButtonSize::Xs)
        .variant(ButtonVariant::Ghost)
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            let Some(engine) = engine.clone() else {
                return;
            };
            let command = command.clone();
            cx.background_spawn(async move { engine.dispatch(command) })
                .detach();
        })
}

fn is_empty_input(input: &str) -> bool {
    let input = input.trim();
    input.is_empty() || input == "{}"
}

fn call_body(
    item: &ThreadItem,
    target_text: &str,
    raw_input: Option<String>,
    ctx: &WorkContext,
) -> Div {
    let ItemContent::ToolCall {
        output,
        is_error,
        detail,
        ..
    } = ctx.thread.content(item)
    else {
        return div();
    };
    let id = &item.id;
    let shown_input = if target_text.is_empty() {
        raw_input
    } else {
        Some(target_text.to_owned())
    };
    let has_input = shown_input.is_some();
    let input_block = shown_input.map(|text| {
        CodeBlock::new(SharedString::from(format!("{id}:input")), text).max_height(px(160.))
    });
    let diff = match detail {
        Some(ToolDetail::FileChange { diff, path, .. }) if !is_error => Some((diff, path)),
        _ => None,
    };
    let rest = match (diff, output) {
        (Some((Some(diff), path)), _) => Some(
            div()
                .when(has_input, |block| {
                    block
                        .border_t_1()
                        .border_color(ctx.colors.border.opacity(0.6))
                })
                .child(DiffView::patch(
                    SharedString::from(format!("{id}:diff")),
                    path.clone(),
                    diff.clone(),
                ))
                .into_any_element(),
        ),
        (Some((None, _)), _) => None,
        (None, Some(output)) if output.trim().is_empty() => None,
        (None, Some(output)) => {
            let (shown, hidden) = head_lines(output, OUTPUT_MAX_LINES, OUTPUT_MAX_CHARS);
            Some(
                CodeBlock::new(SharedString::from(format!("{id}:output")), shown.to_owned())
                    .muted(true)
                    .divided(has_input)
                    .footer((hidden > 0).then(|| crate::m::tool_output_more_lines(hidden).into()))
                    .into_any_element(),
            )
        }
        (None, None) => ctx.thread.live_output(id).map(|live| {
            CodeBlock::new(SharedString::from(format!("{id}:live")), tail(live))
                .muted(true)
                .divided(has_input)
                .into_any_element()
        }),
    };
    div().children(input_block).children(rest)
}

fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.trim_end().lines().collect();
    lines[lines.len().saturating_sub(LIVE_OUTPUT_LINES)..].join("\n")
}
