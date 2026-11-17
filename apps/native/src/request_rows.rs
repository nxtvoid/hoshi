//! Requests in the transcript, once answered or while their panel waits for the user.

use crate::m;
use crate::tool_calls::tool_target;
use crate::transcript::TranscriptView;
use crate::work::WorkContext;
use chrono::{DateTime, Local, TimeZone};
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::*;
use hoshi_core::{
    ApprovalDecision, Command, HandoffDelivery, HandoffSource, ItemContent, ModelChoice,
    ModelSwitch, RequestStatus, Resumption, ThreadItem, TurnFailure,
};
use hoshi_ui::prelude::*;
use std::collections::HashMap;

/// The user asked to sign in to a provider again, which its settings do.
pub struct OpenProviderSettings(pub &'static str);

impl EventEmitter<OpenProviderSettings> for TranscriptView {}

/// An approval a tool call waited on, shown at the end of the call's row.
#[derive(Clone, Debug)]
pub struct CallApproval {
    tool: String,
    status: RequestStatus,
    decision: Option<ApprovalDecision>,
    feedback: Option<String>,
}

/// The approvals of the transcript's tool calls, by the call they held back.
pub fn approvals_by_call(items: &[ThreadItem]) -> HashMap<String, CallApproval> {
    items
        .iter()
        .filter_map(|item| match &item.content {
            ItemContent::Approval {
                tool_call_id: Some(call),
                tool_name,
                status,
                decision,
                feedback,
                ..
            } => Some((
                call.clone(),
                CallApproval {
                    tool: tool_name.clone(),
                    status: *status,
                    decision: *decision,
                    feedback: feedback.clone(),
                },
            )),
            _ => None,
        })
        .collect()
}

/// Whether the user had to approve the call, and what they answered.
pub fn approval_badge(call_id: &str, approval: &CallApproval, colors: &Colors) -> AnyElement {
    let label = match approval.status {
        RequestStatus::Pending => m::request_waiting(),
        _ => approval_label(&approval.tool, approval.status, approval.decision),
    };
    let tone = match (approval.status, approval.decision) {
        (RequestStatus::Pending, _) => colors.status_waiting,
        (RequestStatus::Resolved, Some(ApprovalDecision::Decline | ApprovalDecision::Cancel)) => {
            colors.destructive
        }
        _ => colors.muted_foreground,
    };
    let tip = match &approval.feedback {
        Some(feedback) => format!("{label}\n{feedback}"),
        None => label,
    };
    div()
        .id(SharedString::from(format!("approval-{call_id}")))
        .flex_none()
        .child(Icon::new(IconName::ShieldUser).size_3p5().text_color(tone))
        .with_tooltip(Tooltip::new(tip).side_offset(px(10.)))
        .into_any_element()
}

/// What the user answered an approval, said with the tool's name.
pub fn approval_label(
    tool: &str,
    status: RequestStatus,
    decision: Option<ApprovalDecision>,
) -> String {
    match (status, decision) {
        (RequestStatus::Pending, _) => m::request_approval_title(tool),
        (RequestStatus::Cancelled, _) => m::request_unanswered(tool),
        (_, Some(ApprovalDecision::AcceptForSession)) => m::request_allowed_session(tool),
        (_, Some(ApprovalDecision::Decline)) => m::request_declined(tool),
        (_, Some(ApprovalDecision::Cancel)) => m::request_stopped(tool),
        _ => m::request_allowed(tool),
    }
}

/// What the user answered an approval that held back no call, as an MCP server's own question,
/// which cancelling answers without stopping the turn.
fn standalone_approval_label(
    tool: &str,
    status: RequestStatus,
    decision: Option<ApprovalDecision>,
) -> String {
    match (status, decision) {
        (RequestStatus::Resolved, Some(ApprovalDecision::Cancel)) => m::request_cancelled(tool),
        _ => approval_label(tool, status, decision),
    }
}

fn declined(decision: Option<ApprovalDecision>) -> bool {
    matches!(
        decision,
        Some(ApprovalDecision::Decline | ApprovalDecision::Cancel)
    )
}

fn status_icon(status: RequestStatus, declined: bool, colors: &Colors) -> Icon {
    let (name, color) = match status {
        RequestStatus::Pending => (IconName::Clock, colors.status_waiting),
        RequestStatus::Cancelled => (IconName::X, colors.muted_foreground),
        RequestStatus::Resolved if declined => (IconName::X, colors.muted_foreground),
        RequestStatus::Resolved => (IconName::Check, colors.muted_foreground),
    };
    Icon::new(name).size_4().flex_none().text_color(color)
}

fn title_row(icon: Icon, label: String, colors: &Colors) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .min_h_6()
        .text_sm()
        .text_color(colors.muted_foreground)
        .child(icon)
        .child(
            div()
                .flex_none()
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors.foreground)
                .child(label),
        )
}

fn feedback(text: &str, colors: &Colors) -> Div {
    div()
        .ml_6()
        .pl_2p5()
        .border_l_1()
        .border_color(colors.border)
        .text_sm()
        .text_color(colors.muted_foreground)
        .child(text.to_owned())
}

pub fn request_row(item: &ThreadItem, ctx: &WorkContext, cx: &App) -> AnyElement {
    let colors = ctx.colors;
    match &item.content {
        ItemContent::Approval {
            tool_name,
            input,
            status,
            decision,
            feedback: said,
            ..
        } => div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                title_row(
                    status_icon(*status, declined(*decision), colors),
                    standalone_approval_label(tool_name, *status, *decision),
                    colors,
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_xs()
                        .child(tool_target(tool_name, input, ctx.thread.cwd.as_deref())),
                ),
            )
            .when_some(said.as_deref(), |row, said| {
                row.child(feedback(said, colors))
            })
            .into_any_element(),
        ItemContent::Questions {
            questions,
            status,
            answers,
        } => {
            let icon = if *status == RequestStatus::Resolved {
                Icon::new(IconName::MessageCircleQuestionMark)
                    .size_4()
                    .flex_none()
                    .text_color(colors.muted_foreground)
            } else {
                status_icon(*status, false, colors)
            };
            let label = match status {
                RequestStatus::Pending => m::request_waiting(),
                RequestStatus::Cancelled => m::request_questions_unanswered(),
                RequestStatus::Resolved => m::request_answered(),
            };
            let pairs = questions.iter().enumerate().map(|(index, question)| {
                let answer = answers
                    .as_ref()
                    .and_then(|answers| answers.get(index))
                    .filter(|answer| !answer.is_empty());
                div()
                    .child(
                        div()
                            .text_color(colors.muted_foreground)
                            .child(question.question.clone()),
                    )
                    .when_some(answer, |pair, answer| {
                        pair.child(div().text_color(colors.foreground).child(answer.clone()))
                    })
            });
            div()
                .flex()
                .flex_col()
                .gap_1p5()
                .child(title_row(icon, label, colors))
                .child(
                    div()
                        .ml_2()
                        .pl_4()
                        .border_l_1()
                        .border_color(colors.border)
                        .flex()
                        .flex_col()
                        .gap_1p5()
                        .text_sm()
                        .children(pairs),
                )
                .into_any_element()
        }
        ItemContent::ProposedPlan {
            plan,
            status,
            decision,
            feedback: said,
        } => {
            let label = match (status, decision) {
                (RequestStatus::Pending, _) => m::request_plan_title(),
                (RequestStatus::Cancelled, _) => m::request_plan_unanswered(),
                (_, Some(ApprovalDecision::Accept | ApprovalDecision::AcceptForSession)) => {
                    m::request_plan_accepted()
                }
                _ => m::request_plan_sent_back(),
            };
            let open = ctx.open.contains(&item.id);
            let body = div()
                .id(SharedString::from(format!("plan-body-{}", item.id)))
                .max_h(px(384.))
                .overflow_y_scroll()
                .rounded_md()
                .bg(colors.muted.opacity(0.4))
                .px_3()
                .py_2()
                .text_sm()
                .child(crate::markdown::markdown(
                    SharedString::from(format!("plan-{}", item.id)),
                    plan.clone(),
                    colors,
                ));
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    Disclosure::new(SharedString::from(format!("plan-{}", item.id)))
                        .open(open)
                        .on_toggle(ctx.toggle(&item.id))
                        .child(status_icon(*status, declined(*decision), colors))
                        .child(
                            div()
                                .flex_none()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(colors.foreground)
                                .child(label),
                        )
                        .when(open, |disclosure| disclosure.nested(true).body(body)),
                )
                .when_some(said.as_deref(), |row, said| {
                    row.child(feedback(said, colors))
                })
                .into_any_element()
        }
        ItemContent::SecretRequest {
            label,
            status,
            saved,
            ..
        } => {
            let text = match (status, saved) {
                (RequestStatus::Pending, _) => m::secret_request_waiting(label),
                (RequestStatus::Cancelled, _) => m::secret_request_unanswered(label),
                (_, Some(true)) => m::secret_request_saved(label),
                _ => m::secret_request_declined(label),
            };
            title_row(
                status_icon(*status, *saved == Some(false), colors),
                text,
                colors,
            )
            .into_any_element()
        }
        _ => div().into_any_element(),
    }
}

/// Where the user stopped a turn: a rule across the lane.
/// Where a turn stopped before it finished; `cut` when a restart of hoshi stopped it.
pub fn turn_interrupted(cut: bool, colors: &Colors) -> AnyElement {
    let rule = || div().h_px().flex_1().bg(colors.border);
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
                .items_center()
                .gap_1p5()
                .child(Icon::new(IconName::CircleStop).size_3p5())
                .child(if cut {
                    m::cut_by_restart()
                } else {
                    m::turn_interrupted()
                }),
        )
        .child(rule())
        .into_any_element()
}

/// The provider's name as the screens show it.
pub fn provider_name(provider: &str) -> String {
    crate::providers::known(provider).map_or_else(|| provider.to_owned(), |known| (known.name)())
}

/// Why the turn failed, for the user: `provider` is the thread's, and a usage limit's reset
/// shows in `now`'s time zone.
pub fn failure_message<Tz: TimeZone>(
    reason: &TurnFailure,
    provider: &str,
    now: &DateTime<Tz>,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let name = provider_name(provider);
    match reason {
        TurnFailure::ProviderDisabled => m::turn_failed_provider_disabled(),
        TurnFailure::SpawnFailed { .. } => m::turn_failed_spawn(),
        TurnFailure::ProcessExited { .. } => m::turn_failed_process_exited(),
        TurnFailure::ProviderError { .. } => m::turn_failed_provider_error(),
        TurnFailure::UsageLimited { resets_at, .. } => {
            let ahead = resets_at
                .as_deref()
                .filter(|at| DateTime::parse_from_rfc3339(at).is_ok_and(|at| at > *now));
            match ahead.and_then(|at| crate::usage::limits::moment_at(at, now)) {
                Some(moment) => m::turn_failed_usage_limited_resets(
                    name,
                    crate::usage::limits::reset_when(moment),
                ),
                None => m::turn_failed_usage_limited(name),
            }
        }
        TurnFailure::Overloaded { .. } => m::turn_failed_overloaded(name),
        TurnFailure::AuthenticationFailed { .. } => m::turn_failed_authentication(name),
        TurnFailure::ContextFull { .. } => m::turn_failed_context_full(),
        TurnFailure::ContextHandoffTooLarge => m::turn_failed_context_handoff_too_large(),
        TurnFailure::StartupTimedOut { .. } => m::turn_failed_startup_timed_out(name),
        TurnFailure::ResumeFailed { .. } => m::turn_failed_resume_failed(name),
        TurnFailure::BackgroundWorkRunning => m::turn_failed_background_work(name),
    }
}

/// A failed turn's reason, with what can be done about it while it is the latest thing the
/// thread did: sign in again, or spend a banked reset.
pub fn failure_row(
    item: &ThreadItem,
    reason: &TurnFailure,
    ctx: &WorkContext,
    cx: &App,
) -> AnyElement {
    let colors = ctx.colors;
    let provider = crate::subagents::thread_provider(&item.thread_id, cx);
    let latest = !ctx.working && nothing_since(item, &ctx.thread.items);
    let known = crate::providers::known(&provider).map(|known| known.id);
    let action = match (reason, known) {
        (TurnFailure::AuthenticationFailed { .. }, Some(provider)) if latest => {
            let view = ctx.view.clone();
            let id = format!("sign-in-{}", item.id);
            Some(
                div()
                    .debug_selector({
                        let id = id.clone();
                        move || id.clone()
                    })
                    .child(
                        Button::new(SharedString::from(id))
                            .label(m::turn_failed_sign_in())
                            .variant(ButtonVariant::Outline)
                            .size(ButtonSize::Xs)
                            .on_click(move |_, _, cx| {
                                view.update(cx, |_, cx| cx.emit(OpenProviderSettings(provider)))
                                    .ok();
                            }),
                    )
                    .into_any_element(),
            )
        }
        (TurnFailure::UsageLimited { resets_at, .. }, Some(provider)) if latest => {
            let actions: Vec<AnyElement> = crate::usage::spend_reset_button(&item.id, provider, cx)
                .into_iter()
                .chain(limit_recovery_buttons(item, resets_at.as_deref(), ctx, cx))
                .collect();
            (!actions.is_empty()).then(|| {
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_1p5()
                    .children(actions)
                    .into_any_element()
            })
        }
        _ => None,
    };
    let tone = match reason {
        TurnFailure::UsageLimited { .. } => colors.warning,
        _ => colors.destructive,
    };
    let selector = format!("turn-failed-{}", item.id);
    div()
        .debug_selector(move || selector.clone())
        .flex()
        .flex_col()
        .items_start()
        .gap_1p5()
        .text_sm()
        .text_color(tone)
        .child(failure_message(reason, &provider, &Local::now()))
        .children(action)
        .into_any_element()
}

/// What the user can do while a usage limit's reset is ahead: have the thread go on by
/// itself then, and snooze it until then, or undo either.
fn limit_recovery_buttons(
    item: &ThreadItem,
    resets_at: Option<&str>,
    ctx: &WorkContext,
    cx: &App,
) -> Vec<AnyElement> {
    let ahead = resets_at
        .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
        .is_some_and(|at| at > Local::now());
    let workspace = crate::markdown::workspace(cx);
    let (Some(workspace), true, false) = (workspace, ahead, item.thread_id.contains(':')) else {
        return Vec::new();
    };
    let engine = workspace.read(cx).engine.clone();
    let thread_id = item.thread_id.clone();
    let resuming = engine.usage_limit_resume(&thread_id).is_some();
    let snoozed = engine.usage_limit_snoozed(&thread_id);
    let button = |key: &str, label: String, act: LimitAction| {
        let id = format!("{key}-{}", item.id);
        let (engine, thread_id, view) = (engine.clone(), thread_id.clone(), ctx.view.clone());
        div()
            .debug_selector({
                let id = id.clone();
                move || id.clone()
            })
            .child(
                Button::new(SharedString::from(id))
                    .label(label)
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::Xs)
                    .on_click(move |_, _, cx| {
                        let (engine, thread_id, view) =
                            (engine.clone(), thread_id.clone(), view.clone());
                        cx.spawn(async move |cx| {
                            cx.background_spawn(async move { act.run(&engine, &thread_id) })
                                .await;
                            view.update(cx, |_, cx| cx.notify()).ok();
                        })
                        .detach();
                    }),
            )
            .into_any_element()
    };
    vec![
        if resuming {
            button(
                "limit-cancel-resume",
                m::usage_limit_cancel_resume(),
                LimitAction::CancelResume,
            )
        } else {
            button(
                "limit-resume",
                m::usage_limit_resume_at_reset(),
                LimitAction::Resume,
            )
        },
        if snoozed {
            button("limit-wake", m::usage_limit_wake_now(), LimitAction::Wake)
        } else {
            button(
                "limit-snooze",
                m::usage_limit_snooze_until_reset(),
                LimitAction::Snooze,
            )
        },
    ]
}

#[derive(Clone, Copy)]
enum LimitAction {
    Resume,
    CancelResume,
    Snooze,
    Wake,
}

impl LimitAction {
    fn run(self, engine: &hoshi_engine::Engine, thread_id: &str) {
        match self {
            Self::Resume => {
                engine.resume_at_usage_limit_reset(thread_id);
            }
            Self::CancelResume => engine.cancel_usage_limit_resume(thread_id),
            Self::Snooze => {
                engine.snooze_until_usage_limit_resets(thread_id);
            }
            Self::Wake => {
                let woken = engine.dispatch(Command::ThreadWake {
                    thread_id: thread_id.to_owned(),
                });
                if let Err(error) = woken {
                    eprintln!("hoshi: could not wake a thread: {error}");
                }
            }
        }
    }
}

/// Where the agent was handed context it did not see itself: from the provider the thread
/// ran on before, the thread a fork came from, or a fork merged back.
pub fn context_handoff(
    item: &ThreadItem,
    sources: &[HandoffSource],
    delivery: HandoffDelivery,
    colors: &Colors,
    cx: &App,
) -> AnyElement {
    let title = |thread_id: &str| {
        crate::markdown::workspace(cx)
            .and_then(|workspace| {
                workspace
                    .read(cx)
                    .model
                    .thread(thread_id)
                    .map(|thread| thread.title.clone())
            })
            .filter(|title| !title.is_empty())
            .unwrap_or_else(m::untitled_thread)
    };
    let text = handoff_text(sources, title);
    let icon = match sources.first() {
        Some(HandoffSource::Fork { .. }) => IconName::GitFork,
        Some(HandoffSource::MergeBack { .. }) => IconName::GitMerge,
        _ => IconName::ArrowRightLeft,
    };
    let tone = if delivery == HandoffDelivery::Failed {
        colors.destructive
    } else {
        colors.muted_foreground
    };
    let rule = || div().h_px().flex_1().bg(colors.border);
    let selector = format!("handoff-{}", item.id);
    div()
        .debug_selector(move || selector.clone())
        .flex()
        .items_center()
        .gap_3()
        .text_xs()
        .text_color(tone)
        .child(rule())
        .child(
            div()
                .flex()
                .min_w_0()
                .items_center()
                .gap_1p5()
                .child(Icon::new(icon).size_3p5().flex_none())
                .child(div().min_w_0().truncate().child(text)),
        )
        .child(rule())
        .into_any_element()
}

/// What a change of model or effort between two turns says, as `Opus 5.5 → Sonnet 5.5` or
/// `Medium → High`: only what changed once each side's defaults are known from `workspace`'s
/// catalogs, or as recorded without them. `None` when nothing did.
pub fn model_switch_text(
    switch: &ModelSwitch,
    workspace: Option<&crate::state::Workspace>,
) -> Option<String> {
    let resolve = |choice: &ModelChoice| {
        let model = workspace.and_then(|workspace| {
            workspace.chosen_model(&choice.provider, choice.model.as_deref())
        });
        let slug = model
            .map(|model| model.slug.clone())
            .or_else(|| choice.model.clone());
        let name = model
            .map(|model| model.short_name.clone())
            .or_else(|| choice.model.clone())
            .unwrap_or_else(|| provider_name(&choice.provider));
        let effort = choice
            .effort
            .or_else(|| model.and_then(|model| model.default_effort));
        (slug, name, effort)
    };
    let (from, to) = (resolve(&switch.from), resolve(&switch.to));
    let model_changed = from.0 != to.0;
    let effort_changed = from.2.is_some() && to.2.is_some() && from.2 != to.2;
    if !model_changed && !effort_changed {
        return None;
    }
    let side = |(_, name, effort): &(Option<String>, String, Option<hoshi_core::Effort>)| {
        let effort = effort
            .filter(|_| effort_changed)
            .map(crate::composer::effort_label);
        [model_changed.then(|| name.clone()), effort]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ")
    };
    Some(m::model_switch(side(&from), side(&to)))
}

/// A line across the transcript over the message whose turn changed model or effort.
pub fn model_switch_divider(item: &ThreadItem, text: String, colors: &Colors) -> AnyElement {
    let rule = || div().h_px().flex_1().bg(colors.border);
    let selector = format!("model-switch-{}", item.id);
    div()
        .debug_selector(move || selector.clone())
        .flex()
        .items_center()
        .gap_3()
        .text_xs()
        .text_color(colors.muted_foreground)
        .child(rule())
        .child(div().min_w_0().truncate().child(text))
        .child(rule())
        .into_any_element()
}

/// What a handoff's line says of its sources; `title` names a thread by its id.
pub fn handoff_text(sources: &[HandoffSource], title: impl Fn(&str) -> String) -> String {
    sources
        .iter()
        .map(|source| match source {
            HandoffSource::ProviderSwitch { from, to, .. } => {
                m::context_handoff(provider_name(&from.provider), provider_name(&to.provider))
            }
            HandoffSource::Fork { thread_id } => m::context_handoff_fork(title(thread_id)),
            HandoffSource::MergeBack { fork_thread_id } => {
                m::context_handoff_merge_back(title(fork_thread_id))
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Whether nothing the user or the agent said came after `item`.
fn nothing_since(item: &ThreadItem, items: &[ThreadItem]) -> bool {
    items
        .iter()
        .skip_while(|known| known.id != item.id)
        .skip(1)
        .all(|later| {
            !matches!(
                later.content,
                ItemContent::UserMessage { .. }
                    | ItemContent::AssistantMessage { .. }
                    | ItemContent::TurnFailed { .. }
                    | ItemContent::TurnInterrupted
                    | ItemContent::TurnCut
            )
        })
}

/// A line in a turn about how it ran, such as an MCP server that failed to start.
fn notice(id: String, icon: IconName, text: String, colors: &Colors) -> AnyElement {
    notice_row(id, icon, div().min_w_0().child(text), colors)
}

/// A notice's icon beside `body`, level with its first line.
fn notice_row(id: String, icon: IconName, body: impl IntoElement, colors: &Colors) -> AnyElement {
    div()
        .debug_selector(move || id.clone())
        .py_0p5()
        .flex()
        .min_w_0()
        .items_start()
        .gap_2()
        .text_sm()
        .text_color(colors.muted_foreground)
        .child(
            div()
                .flex_none()
                .h_5()
                .flex()
                .items_center()
                .child(Icon::new(icon).size_4()),
        )
        .child(body)
        .into_any_element()
}

pub fn model_rerouted(
    item: &ThreadItem,
    model: &str,
    reason: Option<&str>,
    colors: &Colors,
    cx: &App,
) -> AnyElement {
    let provider = provider_name(&crate::subagents::thread_provider(&item.thread_id, cx));
    let text = match reason.filter(|reason| !reason.trim().is_empty()) {
        Some(reason) => m::model_rerouted_reason(model, provider, reason.trim()),
        None => m::model_rerouted(model, provider),
    };
    notice(format!("notice-{}", item.id), IconName::Bot, text, colors)
}

pub fn resumed_text(resumed: Resumption) -> String {
    match resumed {
        Resumption::Restart => m::resumed_after_restart(),
        Resumption::RestartEndedWork => m::restart_ended_background_work(),
        Resumption::UsageLimitReset => m::resumed_after_usage_limit(),
    }
}

/// The message hoshi opened a turn with as it went on with the thread by itself.
pub fn resumed_turn(item: &ThreadItem, resumed: Resumption, colors: &Colors) -> AnyElement {
    let icon = match resumed {
        Resumption::UsageLimitReset => IconName::Clock,
        Resumption::Restart | Resumption::RestartEndedWork => IconName::RotateCcw,
    };
    notice(
        format!("notice-{}", item.id),
        icon,
        resumed_text(resumed),
        colors,
    )
}

pub fn provider_recovered_text(provider: &str, attempt: u32, max_attempts: Option<u32>) -> String {
    let progress = match max_attempts {
        Some(max) => format!("{attempt}/{max}"),
        None => attempt.to_string(),
    };
    m::provider_recovered(progress, provider)
}

pub fn provider_recovered(
    item: &ThreadItem,
    attempt: u32,
    max_attempts: Option<u32>,
    colors: &Colors,
    cx: &App,
) -> AnyElement {
    let provider = provider_name(&crate::subagents::thread_provider(&item.thread_id, cx));
    notice(
        format!("notice-{}", item.id),
        IconName::RefreshCw,
        provider_recovered_text(&provider, attempt, max_attempts),
        colors,
    )
}

pub fn mcp_server_failed_text(name: &str) -> String {
    if name == hoshi_engine::MCP_SERVER_NAME {
        m::mcp_server_failed_hoshi()
    } else {
        m::mcp_server_failed(name)
    }
}

/// What failed, with the provider's error below it as written, line breaks and all.
pub fn mcp_server_failed(
    item: &ThreadItem,
    name: &str,
    error: &str,
    colors: &Colors,
    cx: &App,
) -> AnyElement {
    let id = format!("notice-{}", item.id);
    let detail = format!("{id}-detail");
    let error = error.trim();
    let body = div()
        .min_w_0()
        .flex()
        .flex_col()
        .gap_1()
        .child(mcp_server_failed_text(name))
        .when(!error.is_empty(), |this| {
            this.child(
                div()
                    .debug_selector(move || detail.clone())
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_xs()
                    .child(error.to_owned()),
            )
        });
    notice_row(id, IconName::Plug, body, colors)
}

pub fn hook_stopped_text(event: &str, source: Option<&str>, reason: &str) -> String {
    match source {
        Some(source) => m::turn_stopped_by_hook_from(event, reason, source),
        None => m::turn_stopped_by_hook(event, reason),
    }
}

pub fn hook_stopped(
    item: &ThreadItem,
    event: &str,
    source: Option<&str>,
    reason: &str,
    colors: &Colors,
) -> AnyElement {
    notice(
        format!("notice-{}", item.id),
        IconName::CircleStop,
        hook_stopped_text(event, source, reason),
        colors,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        approval_label, failure_message, handoff_text, hook_stopped_text, mcp_server_failed_text,
        provider_recovered_text, resumed_text, standalone_approval_label,
    };
    use crate::m;
    use chrono::{TimeZone, Utc};
    use hoshi_core::{ApprovalDecision, RequestStatus, Resumption, TurnFailure};

    #[test]
    fn each_failure_says_what_happened_and_what_to_do() {
        let now = Utc.with_ymd_and_hms(2026, 11, 15, 10, 0, 0).unwrap();
        let say = |reason: TurnFailure| failure_message(&reason, "codex", &now);
        let detail = || String::from("raw");

        assert_eq!(
            say(TurnFailure::UsageLimited {
                detail: detail(),
                resets_at: Some("2026-11-15T15:30:00Z".into()),
            }),
            "Codex usage limit reached. Resets today at 3:30 PM."
        );
        for resets_at in [None, Some("2026-11-15T09:00:00Z".into())] {
            assert_eq!(
                say(TurnFailure::UsageLimited {
                    detail: detail(),
                    resets_at,
                }),
                m::turn_failed_usage_limited("Codex")
            );
        }
        assert_eq!(
            say(TurnFailure::Overloaded { detail: detail() }),
            "Codex is overloaded right now. Try again shortly."
        );
        assert_eq!(
            say(TurnFailure::AuthenticationFailed { detail: detail() }),
            m::turn_failed_authentication("Codex")
        );
        assert!(say(TurnFailure::ContextFull { detail: detail() }).contains("/compact"));
        assert_eq!(
            say(TurnFailure::ContextHandoffTooLarge),
            "The request leaves too little context for a provider handoff. Shorten the request or use a larger-context model."
        );
        assert_eq!(
            say(TurnFailure::StartupTimedOut { detail: detail() }),
            m::turn_failed_startup_timed_out("Codex")
        );
        assert!(say(TurnFailure::ResumeFailed { detail: detail() }).contains("Nothing was lost"));
        assert_eq!(
            say(TurnFailure::BackgroundWorkRunning),
            m::turn_failed_background_work("Codex")
        );
        assert_eq!(
            failure_message(
                &TurnFailure::Overloaded { detail: detail() },
                "claudeCode",
                &now
            ),
            m::turn_failed_overloaded("Claude Code")
        );
    }

    #[test]
    fn a_handoff_names_where_its_context_came_from() {
        use hoshi_core::{HandoffSource, HandoffStrategy, ModelChoice};
        let choice = |provider: &str| ModelChoice {
            provider: provider.into(),
            model: None,
            effort: None,
            fast_mode: false,
            ultracode: false,
        };
        let title = |id: &str| format!("thread {id}");
        assert_eq!(
            handoff_text(
                &[HandoffSource::ProviderSwitch {
                    from: choice("claudeCode"),
                    to: choice("codex"),
                    strategy: HandoffStrategy::FullThreadSummary,
                }],
                title
            ),
            "Context handoff · Claude Code → Codex"
        );
        assert_eq!(
            handoff_text(
                &[
                    HandoffSource::Fork {
                        thread_id: "a".into()
                    },
                    HandoffSource::MergeBack {
                        fork_thread_id: "b".into()
                    }
                ],
                title
            ),
            "Context from thread a · Merged back from thread b"
        );
    }

    #[test]
    fn a_failed_hoshi_server_says_its_tools_are_missing() {
        assert_eq!(
            mcp_server_failed_text("hoshi"),
            m::mcp_server_failed_hoshi()
        );
        assert_eq!(
            mcp_server_failed_text("github"),
            "MCP server github failed to start"
        );
    }

    #[test]
    fn a_hook_that_stopped_the_turn_says_why() {
        assert_eq!(
            hook_stopped_text("stop", Some("project"), "tests fail"),
            "A stop hook (project) stopped the turn: tests fail"
        );
        assert_eq!(
            hook_stopped_text("preToolUse", None, "no pushes"),
            "A preToolUse hook stopped the turn: no pushes"
        );
    }

    #[test]
    fn a_turn_hoshi_went_on_with_by_itself_says_why() {
        assert_eq!(
            resumed_text(Resumption::Restart),
            "hoshi restarted and resumed this turn"
        );
        assert_eq!(
            resumed_text(Resumption::RestartEndedWork),
            "hoshi restarted and stopped background work"
        );
        assert_eq!(
            resumed_text(Resumption::UsageLimitReset),
            "hoshi resumed this turn once the usage limit reset"
        );
    }

    #[test]
    fn a_recovered_provider_says_how_many_retries_it_took() {
        assert_eq!(
            provider_recovered_text("Claude Code", 3, Some(10)),
            "Claude Code recovered (3/10 retries)"
        );
        assert_eq!(
            provider_recovered_text("Codex", 2, None),
            "Codex recovered (2 retries)"
        );
    }

    #[test]
    fn a_cancelled_mcp_question_reads_as_cancelled_not_stopped() {
        let cancel = Some(ApprovalDecision::Cancel);
        assert_eq!(
            standalone_approval_label("github", RequestStatus::Resolved, cancel),
            "Cancelled github"
        );
        assert_eq!(
            approval_label("Bash", RequestStatus::Resolved, cancel),
            "Stopped at Bash"
        );
        assert_eq!(
            standalone_approval_label(
                "github",
                RequestStatus::Resolved,
                Some(ApprovalDecision::Decline)
            ),
            "Declined github"
        );
    }
}
