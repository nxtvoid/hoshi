//! A subagent in place of the thread that started it: what it was asked, what it did and what
//! it reported. It runs on its own, so a bar stands where the composer would be, with what it
//! runs on, how it stands, a stop button and the way back.

use crate::details::agents::{AgentState, subagent_state};
use crate::m;
use crate::markdown::markdown;
use crate::state::{ThreadState, Workspace};
use crate::subagents::{OpenSubagent, subagent_input};
use crate::transcript::TranscriptView;
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::{Command, ItemContent, ThreadItem};
use hoshi_ui::assets::IconName;
use hoshi_ui::prelude::*;

/// Asks to go back to what started the subagent shown.
pub struct LeaveSubagent;

impl EventEmitter<LeaveSubagent> for SubagentView {}
impl EventEmitter<OpenSubagent> for SubagentView {}

/// The id of a subagent's transcript: its thread's and the call that started it.
pub fn transcript_id(thread_id: &str, tool_use_id: &str) -> String {
    let thread = thread_id.split(':').next().unwrap_or(thread_id);
    format!("{thread}:{tool_use_id}")
}

/// A subagent's items as a turn: an empty question to fold its work under, its items, and
/// its report last unless its last message already says it.
pub fn subagent_items(
    call: &ThreadItem,
    items: &[ThreadItem],
    report: Option<&str>,
) -> Vec<ThreadItem> {
    let thread_id = items
        .first()
        .map_or_else(|| call.thread_id.clone(), |item| item.thread_id.clone());
    let said = items.iter().rev().find_map(|item| match &item.content {
        ItemContent::AssistantMessage { text } => Some(text.trim()),
        _ => None,
    });
    let asked = ThreadItem {
        id: format!("{}:asked", call.id),
        thread_id: thread_id.clone(),
        created_at: call.created_at.clone(),
        content: ItemContent::UserMessage {
            text: String::new(),
            attachments: Vec::new(),
            steer: false,
            task_result: None,
            pull_request_update: None,
            resumed: None,
            rewind: None,
            dropped: false,
            model_switch: None,
        },
    };
    let mut shown = vec![asked];
    shown.extend(items.iter().cloned());
    if let Some(report) = report.filter(|report| Some(report.trim()) != said) {
        shown.push(ThreadItem {
            id: format!("{}:report", call.id),
            thread_id,
            created_at: items
                .last()
                .map_or_else(|| call.created_at.clone(), |item| item.created_at.clone()),
            content: ItemContent::AssistantMessage {
                text: report.to_owned(),
            },
        });
    }
    shown
}

pub struct SubagentView {
    call_id: String,
    parent_title: SharedString,
    /// Where the call that started it is.
    parent: Entity<ThreadState>,
    /// The subagent's own items, as the engine keeps them.
    source: Entity<ThreadState>,
    /// What shows: its items as a turn.
    shown: Entity<ThreadState>,
    transcript: Entity<TranscriptView>,
    prompt_open: bool,
    _subscriptions: Vec<Subscription>,
}

impl SubagentView {
    pub fn new(
        workspace: &Entity<Workspace>,
        parent: Entity<ThreadState>,
        call_id: String,
        parent_title: SharedString,
        cx: &mut Context<Self>,
    ) -> Self {
        let parent_state = parent.read(cx);
        let id = transcript_id(&parent_state.thread_id, &call_id);
        let cwd = parent_state.cwd.clone();
        let engine = parent_state.engine.clone();
        let source = workspace.update(cx, |workspace, cx| {
            workspace.open_subagent(&id, cwd.clone(), cx)
        });
        let shown = cx.new(|_| {
            let mut state = ThreadState::new(&id, Vec::new());
            state.engine = engine;
            state.cwd = cwd;
            state.loading = true;
            state
        });
        let running = Self::running_of(&parent, &call_id, cx);
        let transcript = cx.new(|cx| TranscriptView::new(shown.clone(), running, cx));
        let subscriptions = vec![
            cx.observe(&parent, |this, _, cx| this.sync(cx)),
            cx.observe(&source, |this, _, cx| this.sync(cx)),
            cx.subscribe(&transcript, |_, _, event: &OpenSubagent, cx| {
                cx.emit(OpenSubagent(event.0.clone()))
            }),
        ];
        let mut view = Self {
            call_id,
            parent_title,
            parent,
            source,
            shown,
            transcript,
            prompt_open: false,
            _subscriptions: subscriptions,
        };
        view.sync(cx);
        view
    }

    pub fn shown(&self) -> &Entity<ThreadState> {
        &self.shown
    }

    pub fn title(&self, cx: &App) -> SharedString {
        self.call(cx)
            .map(|call| match &call.content {
                ItemContent::ToolCall { input, .. } => subagent_input(input).description,
                _ => String::new(),
            })
            .filter(|title| !title.is_empty())
            .map_or_else(|| m::subagent().into(), SharedString::from)
    }

    fn call<'a>(&self, cx: &'a App) -> Option<&'a ThreadItem> {
        self.parent
            .read(cx)
            .items
            .iter()
            .find(|item| item.id == self.call_id)
    }

    fn state(&self, cx: &App) -> Option<AgentState> {
        match &self.call(cx)?.content {
            ItemContent::ToolCall {
                output,
                is_error,
                interrupted,
                ..
            } => Some(subagent_state(output.is_some(), *is_error, *interrupted)),
            _ => None,
        }
    }

    fn running_of(parent: &Entity<ThreadState>, call_id: &str, cx: &App) -> bool {
        parent.read(cx).items.iter().any(|item| {
            item.id == call_id
                && matches!(
                    &item.content,
                    ItemContent::ToolCall {
                        output: None,
                        interrupted: false,
                        ..
                    }
                )
        })
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        let Some(call) = self.call(cx).cloned() else {
            return;
        };
        let (report, running) = match &call.content {
            ItemContent::ToolCall {
                output,
                interrupted,
                ..
            } => (
                output.clone().filter(|_| !*interrupted),
                output.is_none() && !*interrupted,
            ),
            _ => (None, false),
        };
        let source = self.source.read(cx);
        let items = subagent_items(&call, &source.items, report.as_deref());
        let loading = source.loading;
        self.shown.update(cx, |shown, cx| {
            if shown.items != items || shown.loading != loading {
                shown.items = items;
                shown.loading = loading;
                cx.notify();
            }
        });
        self.transcript
            .update(cx, |transcript, cx| transcript.set_working(running, cx));
        cx.notify();
    }

    fn stop(&self, cx: &mut App) {
        let parent = self.parent.read(cx);
        let (Some(engine), Some(call)) = (parent.engine.clone(), self.call(cx)) else {
            return;
        };
        let command = Command::SubagentStop {
            thread_id: call.thread_id.clone(),
            tool_use_id: call.id.clone(),
        };
        cx.background_spawn(async move { engine.dispatch(command) })
            .detach();
    }

    fn bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.hoshi().clone();
        let state = self.state(cx);
        let detail = self.call(cx).and_then(|call| match &call.content {
            ItemContent::ToolCall { detail, .. } => detail.clone(),
            _ => None,
        });
        let run = crate::subagents::run_label(detail.as_ref(), cx);
        let started = self
            .call(cx)
            .map(|call| call.created_at.clone())
            .unwrap_or_default();
        let provider = self
            .call(cx)
            .map_or_else(hoshi_core::default_provider, |call| {
                crate::subagents::thread_provider(&call.thread_id, cx)
            });
        let running = state == Some(AgentState::Running);
        div()
            .px_4()
            .pb_3()
            .child(
                Lane::new().child(
                    div()
                        .min_h_12()
                        .flex()
                        .items_center()
                        .gap_3()
                        .py_2()
                        .pl_4()
                        .pr_2()
                        .rounded_2xl()
                        .border_1()
                        .border_color(colors.border)
                        .bg(colors.card)
                        .text_sm()
                        .shadow_xs()
                        .child(
                            svg()
                                .path(crate::composer::provider_mark(&provider))
                                .size_4()
                                .flex_none()
                                .text_color(crate::composer::provider_tint(&provider, &colors)),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(colors.foreground)
                                .child(run.unwrap_or_else(m::subagent)),
                        )
                        .children(state.map(|state| {
                            crate::subagents::status(state, &started, None, &self.call_id, &colors)
                        }))
                        .child(div().flex_1())
                        .child(
                            div()
                                .flex_none()
                                .text_color(colors.muted_foreground)
                                .child(m::subagent_runs_alone()),
                        )
                        .when(running, |bar| {
                            bar.child(
                                Button::new("stop-subagent")
                                    .label(m::background_task_stop())
                                    .size(ButtonSize::Sm)
                                    .variant(ButtonVariant::Ghost)
                                    .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                            )
                        })
                        .child(
                            Button::new("leave-subagent")
                                .icon(IconName::ArrowUpLeft)
                                .label(m::subagent_open_parent())
                                .size(ButtonSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(LeaveSubagent))),
                        ),
                ),
            )
            .into_any_element()
    }
}

impl Render for SubagentView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let prompt = self.call(cx).and_then(|call| match &call.content {
            ItemContent::ToolCall { input, .. } => {
                Some(subagent_input(input).prompt).filter(|prompt| !prompt.is_empty())
            }
            _ => None,
        });
        let running = self.state(cx) == Some(AgentState::Running);
        let source = self.source.read(cx);
        let empty = !source.loading && source.items.is_empty();
        let rule = || div().h_px().flex_1().bg(colors.border);
        let lead = div()
            .flex()
            .flex_col()
            .gap_2()
            .px_4()
            .pt_4()
            .child(
                Lane::new().child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(rule())
                        .child(
                            div()
                                .flex()
                                .min_w_0()
                                .items_center()
                                .gap_1p5()
                                .child(Icon::new(IconName::Bot).size_3p5().flex_none())
                                .child(div().truncate().child(m::subagent_of(&self.parent_title))),
                        )
                        .child(rule()),
                ),
            )
            .children(prompt.map(|prompt| {
                let open = self.prompt_open;
                Lane::new().child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .id("subagent-prompt")
                                .text_xs()
                                .cursor_pointer()
                                .text_color(colors.muted_foreground)
                                .hover(|style| style.text_color(colors.foreground))
                                .child(m::subagent_prompt())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.prompt_open = !this.prompt_open;
                                    cx.notify();
                                })),
                        )
                        .when(open, |section| {
                            section.child(
                                div()
                                    .id("subagent-prompt-body")
                                    .max_h(px(240.))
                                    .overflow_y_scroll()
                                    .rounded_md()
                                    .bg(colors.muted.opacity(0.4))
                                    .px_3()
                                    .py_2()
                                    .text_xs()
                                    .child(markdown("subagent-prompt-text", prompt, &colors)),
                            )
                        }),
                )
            }));
        let progress = self
            .parent
            .read(cx)
            .streaming
            .iter()
            .filter(|stream| {
                stream.item_id == self.call_id && stream.kind == hoshi_core::StreamKind::ToolOutput
            })
            .map(|stream| crate::subagents::preview(&stream.text, true))
            .rfind(|line| !line.is_empty());
        let note = if running {
            Some(progress.unwrap_or_else(m::working))
        } else if empty {
            Some(m::subagent_no_activity())
        } else {
            None
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(lead)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(self.transcript.clone()),
            )
            .children(note.map(|note| {
                div().px_4().pb_2().child(
                    Lane::new().child(
                        div()
                            .text_sm()
                            .text_color(colors.muted_foreground)
                            .child(note),
                    ),
                )
            }))
            .child(self.bar(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::{subagent_items, transcript_id};
    use hoshi_core::{ItemContent, ThreadItem};

    fn item(id: &str, content: ItemContent) -> ThreadItem {
        ThreadItem {
            id: id.into(),
            thread_id: "t:c".into(),
            created_at: String::new(),
            content,
        }
    }

    #[test]
    fn the_report_comes_last_unless_the_subagent_said_it() {
        assert_eq!(transcript_id("t:x", "c"), "t:c");
        let call = item("c", ItemContent::RunEnded);
        let said = vec![item(
            "a",
            ItemContent::AssistantMessage {
                text: "hola".into(),
            },
        )];
        let ids =
            |items: Vec<ThreadItem>| items.into_iter().map(|item| item.id).collect::<Vec<_>>();
        assert_eq!(
            ids(subagent_items(&call, &said, Some("hola\n"))),
            ["c:asked", "a"]
        );
        assert_eq!(
            ids(subagent_items(&call, &said, Some("done"))),
            ["c:asked", "a", "c:report"]
        );
    }
}
