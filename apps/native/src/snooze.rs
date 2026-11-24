//! Snoozed threads as the user sees them: the menu's choices, the time left, why a snooze was
//! refused, and the lines an open thread shows while snoozed and once it woke.

use crate::m;
use crate::moment;
use crate::state::Workspace;
use chrono::{TimeZone, Utc, Weekday};
use gpui_kit::component::Icon;
use gpui_kit::*;
use hoshi_core::{
    Command, CommandError, SnoozeChoice, SnoozeLabel, SnoozeLeft, SnoozeUnit, is_snoozed,
    shows_woke, snooze_left,
};
use hoshi_ui::prelude::*;
use std::collections::HashSet;

pub fn weekday_name(day: Weekday) -> String {
    match day {
        Weekday::Mon => m::weekday_monday(),
        Weekday::Tue => m::weekday_tuesday(),
        Weekday::Wed => m::weekday_wednesday(),
        Weekday::Thu => m::weekday_thursday(),
        Weekday::Fri => m::weekday_friday(),
        Weekday::Sat => m::weekday_saturday(),
        Weekday::Sun => m::weekday_sunday(),
    }
}

pub fn unit_label(unit: SnoozeUnit) -> String {
    match unit {
        SnoozeUnit::Minutes => m::snooze_unit_minutes(),
        SnoozeUnit::Hours => m::snooze_unit_hours(),
        SnoozeUnit::Days => m::snooze_unit_days(),
    }
}

/// "In 3 hours", "Tomorrow", "Next Friday".
pub fn choice_label(label: SnoozeLabel) -> String {
    match label {
        SnoozeLabel::In(amount) => {
            let count = amount.count as usize;
            match amount.unit {
                SnoozeUnit::Minutes => m::snooze_in_minutes(count),
                SnoozeUnit::Hours => m::snooze_in_hours(count),
                SnoozeUnit::Days => m::snooze_in_days(count),
            }
        }
        SnoozeLabel::ThisMorning => m::snooze_this_morning(),
        SnoozeLabel::ThisAfternoon => m::snooze_this_afternoon(),
        SnoozeLabel::ThisEvening => m::snooze_this_evening(),
        SnoozeLabel::Tomorrow => m::snooze_tomorrow(),
        SnoozeLabel::NextWeek => m::snooze_next_week(),
        SnoozeLabel::NextWeekday(day) => m::snooze_next_weekday(weekday_name(day)),
        SnoozeLabel::Weekday(day) => weekday_name(day),
    }
}

/// When the choice wakes on `zone`'s clock, after its short weekday when the label doesn't name
/// the day: "4:30 PM", "Mon 9:00 AM".
pub fn choice_time<Tz: TimeZone>(choice: &SnoozeChoice, zone: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let at = choice.until.with_timezone(zone);
    if choice.shows_weekday {
        moment::weekday_time(&at)
    } else {
        moment::time(&at)
    }
}

/// "45m", "3h", "2d" or "now".
pub fn left_text(left: SnoozeLeft) -> String {
    match left {
        SnoozeLeft::Now => m::snooze_left_now(),
        SnoozeLeft::Minutes(count) => m::snooze_left_minutes(count),
        SnoozeLeft::Hours(count) => m::snooze_left_hours(count),
        SnoozeLeft::Days(count) => m::snooze_left_days(count),
    }
}

/// "Snoozed, 3h left", or "Snoozed" once its time came.
pub fn status_text(until: &str, now: chrono::DateTime<Utc>) -> String {
    match snooze_left(until, now) {
        SnoozeLeft::Now => m::snoozed_status(),
        left => m::snoozed_left(left_text(left)),
    }
}

/// Why the engine refused to snooze or wake a thread.
pub fn error_message(error: &CommandError) -> String {
    match error {
        CommandError::SnoozeTimePassed { .. } => m::snooze_error_time_passed(),
        CommandError::ThreadArchived { .. } => m::snooze_error_archived(),
        CommandError::ThreadWaitingForUser { .. } => m::snooze_error_waiting(),
        CommandError::MessageQueued { .. } => m::snooze_error_queued(),
        _ => m::error_generic(),
    }
}

/// Tells the user that what they asked of a snooze didn't happen.
pub fn show_failure(title: String, description: String, window: &mut Window, cx: &mut App) {
    DialogLayer::confirm(
        ConfirmRequest::new(title, m::dialog_ok())
            .description(description)
            .tone(ConfirmTone::Warning),
        window,
        cx,
    )
    .detach();
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Line {
    /// "Snoozed, 3h left" with a way to wake it now, over the transcript.
    Status,
    /// "Woke from snooze" until dismissed, over the composer.
    Woke,
}

/// A line of the open thread's snooze.
pub struct SnoozeLine {
    workspace: Entity<Workspace>,
    line: Line,
    /// Threads whose wake the engine is on.
    waking: HashSet<String>,
    _observe: Subscription,
}

impl SnoozeLine {
    pub fn status(workspace: Entity<Workspace>, cx: &mut Context<Self>) -> Self {
        Self::new(workspace, Line::Status, cx)
    }

    pub fn woke(workspace: Entity<Workspace>, cx: &mut Context<Self>) -> Self {
        Self::new(workspace, Line::Woke, cx)
    }

    fn new(workspace: Entity<Workspace>, line: Line, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&workspace, |_, _, cx| cx.notify());
        Self {
            workspace,
            line,
            waking: HashSet::new(),
            _observe: observe,
        }
    }

    fn wake(&mut self, thread_id: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.waking.insert(thread_id.clone()) {
            return;
        }
        cx.notify();
        let engine = self.workspace.read(cx).engine.clone();
        let command = Command::ThreadWake {
            thread_id: thread_id.clone(),
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { engine.dispatch(command) })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.waking.remove(&thread_id);
                cx.notify();
                if let Err(error) = result {
                    show_failure(m::wake_failed(), error_message(&error), window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn dismiss(&self, thread_id: String, cx: &mut Context<Self>) {
        let engine = self.workspace.read(cx).engine.clone();
        cx.background_spawn(async move {
            if let Err(error) = engine.dispatch(Command::ThreadWokeDismiss { thread_id }) {
                eprintln!("hoshi: could not dismiss the woke notice: {error:?}");
            }
        })
        .detach();
    }

    /// What the line says for the open thread now; `None` hides it.
    fn text(&self, cx: &App) -> Option<(String, String)> {
        let workspace = self.workspace.read(cx);
        let thread_id = workspace.open_thread_id(cx)?;
        let thread = workspace.model.thread(&thread_id)?;
        let now = Utc::now();
        match self.line {
            Line::Status => {
                let shown = is_snoozed(&workspace.model, thread, now)
                    && !workspace.engine.usage_limit_snoozed(&thread_id);
                let until = thread.snoozed_until.as_deref().filter(|_| shown)?;
                Some((thread_id.clone(), status_text(until, now)))
            }
            Line::Woke => shows_woke(&workspace.model, thread, now)
                .then(|| (thread_id.clone(), m::woke_from_snooze())),
        }
    }
}

impl Render for SnoozeLine {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = match self.line {
            Line::Status => "snooze-status",
            Line::Woke => "snooze-woke",
        };
        let root = div().id(id);
        let Some((thread_id, text)) = self.text(cx) else {
            return root;
        };
        let colors = cx.hoshi().clone();
        let line = match self.line {
            Line::Status => {
                let waking = self.waking.contains(&thread_id);
                div()
                    .debug_selector(|| "snooze-status-line".into())
                    .mt_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .py_1()
                    .pl_2p5()
                    .pr_1()
                    .rounded_lg()
                    .bg(colors.status_working.opacity(0.1))
                    .text_xs()
                    .child(
                        Icon::new(IconName::AlarmClock)
                            .size_3p5()
                            .flex_none()
                            .text_color(colors.status_working),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(text))
                    .child(
                        Button::new("snooze-wake-now")
                            .label(if waking { m::waking() } else { m::wake_now() })
                            .size(ButtonSize::Xs)
                            .variant(ButtonVariant::Ghost)
                            .disabled(waking)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.wake(thread_id.clone(), window, cx)
                            })),
                    )
            }
            Line::Woke => div()
                .debug_selector(|| "snooze-woke-line".into())
                .mb_2()
                .flex()
                .items_center()
                .gap_2()
                .text_xs()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .px_2()
                        .py_0p5()
                        .rounded_md()
                        .bg(colors.warning.opacity(0.12))
                        .text_color(colors.warning)
                        .child(Icon::new(IconName::AlarmClock).size_3().flex_none())
                        .child(text),
                )
                .child(
                    Button::new("snooze-woke-dismiss")
                        .label(m::dismiss())
                        .size(ButtonSize::Xs)
                        .variant(ButtonVariant::Ghost)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.dismiss(thread_id.clone(), cx)),
                        ),
                ),
        };
        root.bg(colors.background).child(Lane::new().child(line))
    }
}

#[cfg(test)]
mod tests {
    use super::{SnoozeLine, choice_label, choice_time, error_message, status_text};
    use crate::state::{self, Workspace};
    use chrono::{DateTime, FixedOffset, SecondsFormat, Utc, Weekday};
    use gpui_kit::component::theme::ThemeMode;
    use gpui_kit::{
        AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
        TestAppContext, VisualTestContext, Window, WindowOptions, div,
    };
    use hoshi_core::{
        AccessMode, Command, CommandError, SnoozeAmount, SnoozeChoice, SnoozeLabel, SnoozeUnit,
        ThreadWorkspace, default_snooze_presets, snooze_choices,
    };
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn choices_read_as_the_menu_shows_them() {
        let hours = SnoozeLabel::In(SnoozeAmount {
            count: 3,
            unit: SnoozeUnit::Hours,
        });
        let hour = SnoozeLabel::In(SnoozeAmount {
            count: 1,
            unit: SnoozeUnit::Hours,
        });
        assert_eq!(choice_label(hour), "In 1 hour");
        assert_eq!(choice_label(hours), "In 3 hours");
        assert_eq!(choice_label(SnoozeLabel::ThisEvening), "This evening");
        assert_eq!(choice_label(SnoozeLabel::NextWeek), "Next week");
        assert_eq!(
            choice_label(SnoozeLabel::NextWeekday(Weekday::Fri)),
            "Next Friday"
        );
        assert_eq!(
            choice_label(SnoozeLabel::Weekday(Weekday::Wed)),
            "Wednesday"
        );
    }

    #[test]
    fn a_choice_on_another_day_shows_its_weekday_with_the_time() {
        let zone = FixedOffset::east_opt(0).unwrap();
        let now = at("2026-11-13T15:30:00Z").with_timezone(&zone);
        let menu = snooze_choices(&default_snooze_presets(), &now);
        let shown: Vec<(String, String)> = menu
            .iter()
            .map(|choice| (choice_label(choice.label), choice_time(choice, &zone)))
            .collect();
        assert_eq!(
            shown,
            [
                ("In 1 hour".to_owned(), "4:30 PM".to_owned()),
                ("This evening".to_owned(), "6:00 PM".to_owned()),
                ("In 3 hours".to_owned(), "6:30 PM".to_owned()),
                ("Tomorrow".to_owned(), "9:00 AM".to_owned()),
                ("Next week".to_owned(), "Mon 9:00 AM".to_owned()),
            ]
        );
        let unnamed = SnoozeChoice {
            shows_weekday: false,
            ..menu[4].clone()
        };
        assert_eq!(choice_time(&unnamed, &zone), "9:00 AM");
    }

    #[test]
    fn the_status_line_counts_down_to_the_wake() {
        let now = at("2026-11-13T10:00:00Z");
        assert_eq!(status_text("2026-11-13T12:30:00Z", now), "Snoozed, 3h left");
        assert_eq!(
            status_text("2026-11-13T10:45:00Z", now),
            "Snoozed, 45m left"
        );
        assert_eq!(status_text("2026-11-13T09:00:00Z", now), "Snoozed");
    }

    struct Shell {
        lines: [Entity<SnoozeLine>; 2],
    }

    impl Render for Shell {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .children(self.lines.clone())
        }
    }

    fn redraw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        for _ in 0..3 {
            cx.update(|window, cx| window.draw(cx).clear(cx));
            cx.run_until_parked();
        }
    }

    #[gpui_kit::test]
    fn the_open_thread_says_it_is_snoozed_and_then_that_it_woke(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let home =
            std::env::temp_dir().join(format!("hoshi-native-snooze-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let (notify, notifications) = state::channel();
        let engine = Arc::new(hoshi_engine::Engine::open(&home, notify).unwrap());
        engine
            .dispatch(Command::ThreadCreate {
                thread_id: "a".into(),
                project_id: None,
                title: "first".into(),
                workspace: ThreadWorkspace::Local,
                provider: hoshi_core::default_provider(),
                model: None,
                effort: None,
                fast_mode: false,
                ultracode: false,
                access_mode: AccessMode::default(),
                plan_mode: false,
            })
            .unwrap();
        cx.update(|cx| {
            gpui_kit::init(cx);
            hoshi_ui::DialogLayer::init(cx);
            hoshi_ui::theme::apply(ThemeMode::Light, cx);
        });
        let workspace =
            cx.update(|cx| cx.new(|cx| Workspace::new(engine.clone(), notifications, cx)));
        let (window, lines) = cx.update(|cx| {
            let mut lines = None;
            let (window, _) = gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                let shown = [
                    cx.new(|cx| SnoozeLine::status(workspace.clone(), cx)),
                    cx.new(|cx| SnoozeLine::woke(workspace.clone(), cx)),
                ];
                lines = Some(shown.clone());
                cx.new(|_| Shell { lines: shown })
            })
            .unwrap();
            (window, lines.unwrap())
        });
        let mut cx = VisualTestContext::from_window(window, cx);
        workspace.update(&mut cx, |workspace, cx| workspace.open_thread("a", cx));
        let snooze = |until: DateTime<Utc>| {
            engine
                .dispatch(Command::ThreadSnooze {
                    thread_id: "a".into(),
                    until: until.to_rfc3339_opts(SecondsFormat::Millis, true),
                })
                .unwrap();
        };
        let wait_for = |cx: &mut VisualTestContext,
                        what: &str,
                        done: &dyn Fn(&mut VisualTestContext) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                redraw(cx);
                if done(cx) {
                    return;
                }
                assert!(Instant::now() < deadline, "timed out waiting for {what}");
                std::thread::sleep(Duration::from_millis(20));
            }
        };

        snooze(Utc::now() + chrono::Duration::hours(3));
        wait_for(&mut cx, "the status line", &|cx| {
            cx.debug_bounds("snooze-status-line").is_some()
        });
        assert!(cx.debug_bounds("snooze-woke-line").is_none());
        cx.update(|window, cx| lines[0].update(cx, |line, cx| line.wake("a".into(), window, cx)));
        wait_for(&mut cx, "the wake", &|cx| {
            cx.debug_bounds("snooze-status-line").is_none()
        });

        snooze(Utc::now() + chrono::Duration::milliseconds(300));
        std::thread::sleep(Duration::from_millis(400));
        wait_for(&mut cx, "the woke line", &|cx| {
            cx.debug_bounds("snooze-woke-line").is_some()
        });
        cx.update(|_, cx| lines[1].update(cx, |line, cx| line.dismiss("a".into(), cx)));
        wait_for(&mut cx, "the dismissal", &|cx| {
            cx.debug_bounds("snooze-woke-line").is_none()
        });
    }

    #[test]
    fn refusals_say_what_to_do() {
        let thread_id = "t".to_owned();
        assert_eq!(
            error_message(&CommandError::ThreadWaitingForUser {
                thread_id: thread_id.clone()
            }),
            "This thread is waiting on you. Respond to the pending request before snoozing it."
        );
        assert_eq!(
            error_message(&CommandError::SnoozeTimePassed { thread_id }),
            "That snooze time has passed. Choose another time."
        );
    }
}
