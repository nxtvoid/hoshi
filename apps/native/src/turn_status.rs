//! What a running turn shows besides its items, as the web shows it: how long it has worked,
//! where its work will fold, and the commands it left running in the background, at the end.
//! Also the row a background task's end leaves in the transcript.

use crate::format::format_duration;
use crate::m;
use crate::tool_calls::{is_subagent, tool_name, tool_target};
use chrono::{DateTime, Utc};
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::{Command, ItemContent, ThreadBackgroundTasks, ThreadItem, TurnRetry};
use hoshi_engine::Engine;
use hoshi_ui::assets::IconName;
use hoshi_ui::prelude::*;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

/// `m:ss`, or `h:mm:ss` past an hour.
pub fn elapsed(milliseconds: i64) -> String {
    let seconds = milliseconds.max(0) / 1000;
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

fn millis_of(time: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(time)
        .ok()
        .map(|time| time.with_timezone(&Utc).timestamp_millis())
}

/// "Retrying (3/10), next in 30s" while the provider waits to try reaching the model again.
pub fn retry_label(retry: &TurnRetry, now: i64) -> String {
    let progress = match retry.max_attempts {
        Some(max) => format!("{}/{max}", retry.attempt),
        None => retry.attempt.to_string(),
    };
    let left = retry
        .retry_in_ms
        .zip(millis_of(&retry.at))
        .map(|(wait, at)| at.saturating_add(wait as i64) - now)
        .filter(|left| *left >= 1000);
    match left {
        Some(left) => m::turn_retrying_next(format_duration(left), progress),
        None => m::turn_retrying(progress),
    }
}

/// "Working for 8s" since `since`, an item's time, above where the turn's work folds, and how
/// the provider retries while it does.
/// What failed, as the provider said it; an error code such as `rate_limit` reads as words,
/// and one that says nothing is left out.
fn retry_error(error: &str) -> Option<String> {
    let error = error.trim();
    if error.is_empty() || error == "unknown" {
        return None;
    }
    let code = error
        .chars()
        .all(|character| character.is_ascii_lowercase() || character == '_');
    Some(if code {
        error.replace('_', " ")
    } else {
        error.to_owned()
    })
}

/// While a stop is on its way, the row says so and how long it has taken, instead of how long
/// the turn has worked.
pub fn status_row(
    since: Option<&str>,
    stopping: Option<Instant>,
    retry: Option<&TurnRetry>,
    colors: &Colors,
) -> AnyElement {
    let now = now_millis();
    let label = match (stopping, since.and_then(millis_of)) {
        (Some(asked), _) if asked.elapsed() >= Duration::from_secs(1) => {
            m::stopping_for(format_duration(asked.elapsed().as_millis() as i64))
        }
        (Some(_), _) => m::stopping(),
        (None, Some(started)) => m::working_for(format_duration(now - started)),
        (None, None) => m::working(),
    };
    let retry = retry.filter(|_| stopping.is_none());
    let retry = retry.map(|retry| {
        let error = retry.error.as_deref().and_then(retry_error);
        div()
            .debug_selector(|| "turn-retry".into())
            .flex()
            .min_w_0()
            .items_center()
            .gap_1p5()
            .child(div().flex_none().child("·"))
            .child(div().flex_none().child(retry_label(retry, now)))
            .when_some(error, |row, error| {
                row.child(div().flex_none().child("·"))
                    .child(div().min_w_0().truncate().child(error.to_owned()))
            })
    });
    div()
        .pb_2()
        .border_b_1()
        .border_color(colors.border.opacity(0.6))
        .child(
            div()
                .min_h_6()
                .flex()
                .min_w_0()
                .items_center()
                .gap_1p5()
                .text_sm()
                .text_color(colors.muted_foreground)
                .child(div().flex_none().child(label))
                .children(retry),
        )
        .into_any_element()
}

/// The commands the agent left running, each with how long it has run and a stop button. They
/// spin while the turn only waits for them.
pub fn background_rows(
    background: &ThreadBackgroundTasks,
    engine: Option<Arc<Engine>>,
    colors: &Colors,
) -> AnyElement {
    let waiting = background.waiting;
    let now = now_millis();
    let lines = background.tasks.iter().map(|task| {
        let thread_id = background.thread_id.clone();
        let task_id = task.task_id.clone();
        let engine = engine.clone();
        let title: SharedString = if task.description.is_empty() {
            m::background_task().into()
        } else {
            task.description.clone().into()
        };
        let icon = Icon::new(if waiting {
            IconName::LoaderCircle
        } else {
            IconName::SquareTerminal
        })
        .size_4()
        .flex_none();
        div()
            .flex()
            .min_w_0()
            .items_center()
            .gap_2()
            .text_sm()
            .text_color(colors.muted_foreground)
            .child(if waiting {
                icon.with_loop(hoshi_ui::looping(Duration::from_secs(1)), |icon, delta| {
                    icon.transform(Transformation::rotate(percentage(delta)))
                })
                .into_any_element()
            } else {
                icon.into_any_element()
            })
            .child(div().flex_none().child(if waiting {
                m::background_waiting()
            } else {
                m::background_running()
            }))
            .child(div().flex_none().child("·"))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(colors.foreground.opacity(0.8))
                    .child(title),
            )
            .child(div().flex_none().child("·"))
            .child(
                div()
                    .flex_none()
                    .child(elapsed(now - task.started_at as i64)),
            )
            .child(
                Button::new(SharedString::from(format!("stop-{}", task.task_id)))
                    .label(m::background_task_stop())
                    .size(ButtonSize::Xs)
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, _, cx| {
                        let Some(engine) = engine.clone() else {
                            return;
                        };
                        let command = Command::BackgroundTaskStop {
                            thread_id: thread_id.clone(),
                            task_id: task_id.clone(),
                        };
                        cx.background_spawn(async move { engine.dispatch(command) })
                            .detach();
                    }),
            )
    });
    div()
        .flex()
        .flex_col()
        .gap_2()
        .children(lines)
        .into_any_element()
}

/// The command or subagent a background task ran, and whether it finished or failed.
pub fn task_ended_row(
    tool_call_id: Option<&str>,
    failed: bool,
    items: &[ThreadItem],
    cwd: Option<&str>,
    view: WeakEntity<crate::transcript::TranscriptView>,
    colors: &Colors,
) -> AnyElement {
    let call = tool_call_id.and_then(|id| items.iter().find(|item| item.id == id));
    let (subagent, title) = match call.map(|call| &call.content) {
        Some(ItemContent::ToolCall { name, input, .. }) => {
            let target = tool_target(name, input, cwd);
            let title = if target.is_empty() {
                tool_name(name).label
            } else {
                target
            };
            (is_subagent(name), title)
        }
        _ => (false, m::background_task().to_string()),
    };
    let found = call.map(|call| call.id.clone());
    let foreground = colors.foreground;
    div()
        .id(SharedString::from(format!(
            "task-ended-{}",
            tool_call_id.unwrap_or_default()
        )))
        .min_h_6()
        .flex()
        .min_w_0()
        .items_center()
        .gap_2()
        .rounded_md()
        .text_sm()
        .text_color(colors.muted_foreground)
        .when_some(found, |row, id| {
            row.cursor_pointer()
                .hover(move |style| style.text_color(foreground))
                .press_selects_nothing()
                .on_click(move |_, _, cx| {
                    view.update(cx, |view, cx| view.reveal(&id, cx)).ok();
                })
        })
        .child(
            Icon::new(if subagent {
                IconName::Bot
            } else {
                IconName::SquareTerminal
            })
            .size_4()
            .flex_none(),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::MEDIUM)
                .text_color(colors.foreground.opacity(0.8))
                .child(title),
        )
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .text_xs()
                .child(
                    Icon::new(if failed {
                        IconName::TriangleAlert
                    } else {
                        IconName::CircleCheck
                    })
                    .size_3p5()
                    .when(failed, |icon| icon.text_color(colors.destructive)),
                )
                .child(if failed {
                    m::background_task_failed()
                } else {
                    m::background_task_done()
                }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{elapsed, retry_error, retry_label};
    use hoshi_core::TurnRetry;

    #[test]
    fn a_retry_error_code_reads_as_words_and_an_empty_one_is_left_out() {
        assert_eq!(retry_error("rate_limit").as_deref(), Some("rate limit"));
        assert_eq!(retry_error(" unknown ").as_deref(), None);
        assert_eq!(
            retry_error("stream disconnected: 502").as_deref(),
            Some("stream disconnected: 502")
        );
    }

    #[test]
    fn a_retry_counts_its_attempts_and_down_to_the_next() {
        let retry = TurnRetry {
            attempt: 3,
            max_attempts: Some(10),
            retry_in_ms: Some(30_000),
            error: None,
            at: "2026-11-15T10:00:00Z".into(),
        };
        let at = 1_794_736_800_000;
        assert_eq!(retry_label(&retry, at), "Retrying (3/10), next in 30s");
        assert_eq!(
            retry_label(&retry, at + 12_000),
            "Retrying (3/10), next in 18s"
        );
        assert_eq!(retry_label(&retry, at + 30_000), "Retrying (3/10)");
        let endless = TurnRetry {
            max_attempts: None,
            retry_in_ms: None,
            ..retry
        };
        assert_eq!(retry_label(&endless, at), "Retrying (3)");
    }

    #[test]
    fn elapsed_reads_as_a_clock() {
        assert_eq!(elapsed(4_200), "0:04");
        assert_eq!(elapsed(125_000), "2:05");
        assert_eq!(elapsed(3_725_000), "1:02:05");
    }
}
