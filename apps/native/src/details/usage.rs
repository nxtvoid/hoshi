use super::rows::{BESIDE_ROW, icon};
use crate::composer::CROWDED_PERCENT;
use crate::m;
use crate::usage::limits::{
    ExtraTexts, LimitTone, even_pace_left, extra_left_percent, extra_texts, format_span,
    limit_tone, moment_short, remaining_percent, reset_moment, reset_short, reset_when,
    short_window_label, time_left_percent, time_to_reset, window_label,
};
use chrono::{DateTime, Local, TimeZone, Utc};
use gpui_kit::*;
use hoshi_core::{ExtraUsage, SpendUnit, Thread, UsageLimits, UsageWindow, UsageWindowKind};
use hoshi_ui::prelude::*;

/// The columns of the meters, in rems.
const LABEL_WIDTH: f32 = 3.75;
const VALUE_WIDTH: f32 = 2.;
const NOTE_WIDTH: f32 = 2.625;

/// The provider whose limits the thread spends: the one its turn runs on, else the one its next
/// message runs on.
pub fn provider_of(thread: &Thread) -> &str {
    thread
        .pending_model
        .as_ref()
        .filter(|_| thread.active_turn_id.is_none())
        .map_or(thread.provider.as_str(), |choice| choice.provider.as_str())
}

/// The thread's context as its row shows it: how full, what its tooltip says, and the button
/// that compacts it.
pub struct ContextMeter {
    /// `None` when the window's size is unknown: the row then tells the tokens alone.
    pub percent: Option<f32>,
    pub value: String,
    pub lines: Vec<String>,
    pub compact: AnyElement,
}

/// One row per window of `limits`: what is left, where even spending would be, and when it
/// resets; then the plan's extra usage, and how full the thread's context is.
pub fn section(
    limits: Option<&UsageLimits>,
    context: Option<ContextMeter>,
    colors: &Colors,
) -> Div {
    let now = Local::now();
    div()
        .flex()
        .flex_col()
        .children(limits.into_iter().flat_map(|limits| {
            limits
                .windows
                .iter()
                .enumerate()
                .map(|(index, window)| usage_row(index, window, &now, colors))
        }))
        .children(
            limits
                .and_then(|limits| limits.extra_usage.as_ref())
                .map(|extra| extra_row(extra, &now, colors)),
        )
        .children(context.map(|context| context_row(context, colors)))
}

fn window_icon(window: &UsageWindow) -> IconName {
    match (window.kind, &window.model) {
        (UsageWindowKind::Session, _) => IconName::Timer,
        (UsageWindowKind::Weekly, Some(_)) => IconName::Sparkles,
        (UsageWindowKind::Weekly, None) => IconName::Calendar,
    }
}

/// A row of the box: icon, name, a bar with an optional mark on it, the value and a note.
#[allow(clippy::too_many_arguments)]
fn meter(
    id: impl Into<ElementId>,
    name: IconName,
    label: String,
    fill: Option<f32>,
    mark: Option<f32>,
    value: String,
    note: Option<AnyElement>,
    tone: Hsla,
    colors: &Colors,
) -> Stateful<Div> {
    let hover = colors.foreground.opacity(0.05);
    div()
        .id(id)
        .flex()
        .flex_none()
        .h(rems(1.875))
        .items_center()
        .gap(px(10.))
        .rounded_lg()
        .px(px(10.))
        .text_xs()
        .hover(move |style| style.bg(hover))
        .child(icon(name, colors))
        .child(
            div()
                .w(rems(LABEL_WIDTH))
                .flex_none()
                .truncate()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors.foreground.opacity(0.8))
                .child(label),
        )
        .child(match fill {
            None => div().flex_1().min_w_0(),
            Some(fill) => div()
                .relative()
                .flex_1()
                .min_w_0()
                .h(px(3.))
                .rounded_full()
                .bg(colors.muted_foreground.opacity(0.25))
                .when(fill > 0., |this| {
                    this.child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left_0()
                            .min_w(px(6.))
                            .w(relative(fill.min(1.)))
                            .rounded_full()
                            .bg(tone),
                    )
                })
                .when_some(mark, |this, mark| {
                    this.child(
                        div()
                            .absolute()
                            .top(px(-4.))
                            .left(relative(mark.clamp(0., 1.)))
                            .w(px(1.5))
                            .h(px(11.))
                            .rounded_full()
                            .bg(colors.muted_foreground),
                    )
                }),
        })
        .child(
            div()
                .min_w(rems(VALUE_WIDTH))
                .flex_none()
                .flex()
                .justify_end()
                .font_weight(FontWeight::MEDIUM)
                .text_color(tone)
                .child(value),
        )
        .child(
            div()
                .w(rems(NOTE_WIDTH))
                .flex_none()
                .flex()
                .justify_end()
                .text_color(colors.muted_foreground)
                .children(note),
        )
}

fn usage_row(
    index: usize,
    window: &UsageWindow,
    now: &DateTime<Local>,
    colors: &Colors,
) -> impl IntoElement + use<> {
    let left = remaining_percent(window);
    let tone = match limit_tone(left) {
        LimitTone::Low => colors.status_error,
        LimitTone::Warning => colors.warning,
        LimitTone::Normal => colors.foreground.opacity(0.8),
    };
    let pace = time_left_percent(window, now.with_timezone(&Utc)).map(|pace| pace / 100.);
    let window = window.clone();
    meter(
        ("details-usage", index),
        window_icon(&window),
        short_window_label(&window),
        Some(f32::from(left) / 100.),
        pace,
        m::usage_percent(left),
        reset_short(&window, now).map(|reset| reset.into_any_element()),
        tone,
        colors,
    )
    .debug_selector(move || format!("details-usage-{index}"))
    .with_tooltip(
        Tooltip::element(move |_, _| {
            let lines = tooltip_lines(&window, &Local::now());
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .children(lines.into_iter().map(|line| div().child(line)))
        })
        .side(Side::Left)
        .side_offset(px(BESIDE_ROW)),
    )
}

fn context_row(context: ContextMeter, colors: &Colors) -> impl IntoElement + use<> {
    let ContextMeter {
        percent,
        value,
        lines,
        compact,
    } = context;
    let tone = if percent.is_some_and(|used| used > CROWDED_PERCENT) {
        colors.warning
    } else {
        colors.foreground.opacity(0.8)
    };
    meter(
        "details-context",
        IconName::Gauge,
        m::usage_context(),
        percent.map(|used| used / 100.),
        None,
        value,
        Some(compact),
        tone,
        colors,
    )
    .debug_selector(|| "details-context".into())
    .with_tooltip(
        Tooltip::element(move |_, _| {
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .children(lines.iter().cloned().map(|line| div().child(line)))
        })
        .side(Side::Left)
        .side_offset(px(BESIDE_ROW)),
    )
}

/// The plan's extra usage: what is left of its cap on a bar, else the credits left or what was
/// spent, without one.
fn extra_row(
    extra: &ExtraUsage,
    now: &DateTime<Local>,
    colors: &Colors,
) -> impl IntoElement + use<> {
    let left = extra_left_percent(extra);
    let ExtraTexts {
        shown,
        note,
        detail,
    } = extra_texts(extra, now.with_timezone(&Utc));
    let tone = match left.map(limit_tone) {
        Some(LimitTone::Low) => colors.status_error,
        Some(LimitTone::Warning) => colors.warning,
        _ => colors.foreground.opacity(0.8),
    };
    let (name, label, what) = match extra.unit {
        SpendUnit::Money { .. } => (
            IconName::CreditCard,
            m::usage_extra_short(),
            m::usage_extra(),
        ),
        SpendUnit::Credits { .. } => (IconName::Coins, m::usage_credits(), m::usage_credits()),
    };
    let side = match (left, extra.resets_at.as_deref()) {
        (Some(_), Some(at)) => moment_short(at, now),
        (None, _) if extra.balance.is_none() && !extra.unlimited => Some(m::usage_extra_spent()),
        _ => None,
    };
    let amount = match &note {
        Some(note) => m::usage_amount_note(shown.clone(), note.clone()),
        None => shown.clone(),
    };
    let lines: Vec<String> = std::iter::once(m::usage_tooltip_amount(amount, what))
        .chain((!detail.is_empty()).then_some(detail))
        .collect();
    meter(
        "details-extra",
        name,
        label,
        left.map(|left| f32::from(left) / 100.),
        None,
        shown,
        side.map(|side| side.into_any_element()),
        tone,
        colors,
    )
    .debug_selector(|| "details-extra".into())
    .with_tooltip(
        Tooltip::element(move |_, _| {
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .children(lines.iter().cloned().map(|line| div().child(line)))
        })
        .side(Side::Left)
        .side_offset(px(BESIDE_ROW)),
    )
}

fn tooltip_lines<Tz: TimeZone>(window: &UsageWindow, now: &DateTime<Tz>) -> Vec<String>
where
    Tz::Offset: std::fmt::Display,
{
    let utc = now.with_timezone(&Utc);
    let mut lines = vec![m::usage_tooltip_left(
        remaining_percent(window),
        window_label(window),
    )];
    if let (Some(moment), Some(reset)) = (reset_moment(window, now), time_to_reset(window, utc)) {
        lines.push(m::usage_tooltip_resets(m::usage_reset_when(
            format_span(reset),
            reset_when(moment),
        )));
    }
    if let Some(pace) = even_pace_left(window, utc) {
        lines.push(m::usage_tooltip_even_pace(pace));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::{provider_of, tooltip_lines};
    use chrono::{DateTime, TimeZone, Utc};
    use hoshi_core::{ModelChoice, Thread, UsageWindow, UsageWindowKind};

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 13, 12, 0, 0).unwrap()
    }

    fn window(used_percent: u8, resets_at: Option<&str>) -> UsageWindow {
        UsageWindow {
            kind: UsageWindowKind::Weekly,
            model: None,
            used_percent,
            window_minutes: 7 * 24 * 60,
            resets_at: resets_at.map(str::to_owned),
        }
    }

    fn thread(provider: &str) -> Thread {
        serde_json::from_value(serde_json::json!({
            "id": "t",
            "projectId": "p",
            "title": "thread",
            "status": "idle",
            "branch": null,
            "worktreePath": null,
            "sessionId": null,
            "conversationCursor": null,
            "resumeAt": null,
            "watchingPullRequest": false,
            "provider": provider,
            "model": null,
            "effort": null,
            "fastMode": false,
            "ultracode": false,
            "accessMode": "acceptEdits",
            "planMode": false,
            "activeTurnId": null,
            "archived": false,
            "pinned": false,
            "createdAt": "2026-11-13T12:00:00Z",
            "updatedAt": "2026-11-13T12:00:00Z",
        }))
        .unwrap()
    }

    #[test]
    fn the_limits_follow_the_running_turn_then_the_provider_the_next_message_runs_on() {
        let mut thread = thread("claudeCode");
        assert_eq!(provider_of(&thread), "claudeCode");
        thread.pending_model = Some(Box::new(ModelChoice {
            provider: "codex".into(),
            model: None,
            effort: None,
            fast_mode: false,
            ultracode: false,
        }));
        assert_eq!(provider_of(&thread), "codex");
        thread.active_turn_id = Some("turn".into());
        assert_eq!(provider_of(&thread), "claudeCode");
    }

    #[test]
    fn the_tooltip_says_when_a_window_resets_and_how_it_paces_only_when_known() {
        let lines = tooltip_lines(&window(83, None), &now());
        assert_eq!(lines, ["Weekly: 17% left"]);

        let lines = tooltip_lines(&window(83, Some("2026-11-14T12:00:00Z")), &now());
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert_eq!(lines[1], "Resets tomorrow at 12:00 PM · in 1d 0h");
        assert_eq!(lines[2], "Even pace: 14% left by now");
    }
}
