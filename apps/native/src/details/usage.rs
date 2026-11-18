use crate::m;
use crate::usage::limits::{
    LimitTone, even_pace_left, format_span, limit_tone, remaining_percent, reset_moment,
    reset_when, time_to_reset, window_label,
};
use chrono::{DateTime, Local, TimeZone, Utc};
use gpui_kit::*;
use hoshi_core::{Thread, UsageLimits, UsageWindow};
use hoshi_ui::prelude::*;

const LABEL_WIDTH: f32 = 112.;
const VALUE_WIDTH: f32 = 32.;

/// The provider whose limits the thread spends: the one its turn runs on, else the one its next
/// message runs on.
pub fn provider_of(thread: &Thread) -> &str {
    thread
        .pending_model
        .as_ref()
        .filter(|_| thread.active_turn_id.is_none())
        .map_or(thread.provider.as_str(), |choice| choice.provider.as_str())
}

/// One row per window of `limits`: what is left, and on hover when it resets.
pub fn section(limits: &UsageLimits, colors: &Colors) -> Div {
    div().flex().flex_col().py_1().children(
        limits
            .windows
            .iter()
            .enumerate()
            .map(|(index, window)| usage_row(index, window, colors)),
    )
}

fn usage_row(index: usize, window: &UsageWindow, colors: &Colors) -> impl IntoElement + use<> {
    let left = remaining_percent(window);
    let tone = match limit_tone(left) {
        LimitTone::Low => colors.status_error,
        LimitTone::Warning => colors.warning,
        LimitTone::Normal => colors.foreground.opacity(0.8),
    };
    let hover = colors.foreground.opacity(0.05);
    let window = window.clone();
    div()
        .id(("details-usage", index))
        .debug_selector(move || format!("details-usage-{index}"))
        .flex()
        .flex_none()
        .h_6()
        .items_center()
        .gap_3()
        .rounded_lg()
        .px(px(10.))
        .text_xs()
        .hover(move |style| style.bg(hover))
        .child(
            div()
                .w(px(LABEL_WIDTH))
                .flex_none()
                .truncate()
                .text_color(colors.muted_foreground)
                .child(window_label(&window)),
        )
        .child(
            div()
                .relative()
                .flex_1()
                .min_w_0()
                .h(px(3.))
                .rounded_full()
                .bg(colors.muted_foreground.opacity(0.25))
                .when(left > 0, |this| {
                    this.child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left_0()
                            .min_w(px(6.))
                            .w(relative(f32::from(left) / 100.))
                            .rounded_full()
                            .bg(tone),
                    )
                }),
        )
        .child(
            div()
                .w(px(VALUE_WIDTH))
                .flex_none()
                .flex()
                .justify_end()
                .font_weight(FontWeight::MEDIUM)
                .text_color(tone)
                .child(m::usage_percent(left)),
        )
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
            .side_offset(px(16.)),
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
