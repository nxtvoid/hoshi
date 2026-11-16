//! Snoozing from the sidebar: the menu of choices, a time of the user's own, waking, and moving
//! on from the open thread once it is snoozed.

use super::Sidebar;
use super::prefs;
use crate::calendar::{MonthCalendar, month_of};
use crate::m;
use crate::sections::{Section, SectionId};
use crate::snooze::{choice_label, choice_time, error_message, show_failure, unit_label};
use crate::toast::{self, Toast, ToastAction};
use chrono::{DateTime, Days, Local, NaiveDate, SecondsFormat, Utc};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::*;
use hoshi_core::{
    Command, CustomSnooze, SnoozeChoice, SnoozePreset, SnoozeUnit, custom_snooze_until, is_snoozed,
    pick_snooze_choice, snooze_choices,
};
use hoshi_ui::prelude::*;
use std::collections::HashSet;

const UNITS: [SnoozeUnit; 3] = [SnoozeUnit::Minutes, SnoozeUnit::Hours, SnoozeUnit::Days];

/// What undoing a snooze does: put back the snooze the thread was in, while it is still ahead,
/// or wake it.
pub(super) fn undo_command(
    thread_id: &str,
    restore: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Command {
    match restore.filter(|until| *until > now) {
        Some(until) => Command::ThreadSnooze {
            thread_id: thread_id.to_owned(),
            until: until.to_rfc3339_opts(SecondsFormat::Millis, true),
        },
        None => Command::ThreadWake {
            thread_id: thread_id.to_owned(),
        },
    }
}

/// The thread to show once `current` is snoozed: the next one listed, wrapping around, that
/// isn't snoozed or done. `None` when there is none, or `current` isn't listed.
pub(super) fn next_unparked(
    sections: &[Section],
    is_open: impl Fn(SectionId) -> bool,
    current: &str,
) -> Option<String> {
    let parked: HashSet<&str> = sections
        .iter()
        .filter(|section| matches!(section.id, SectionId::Snoozed | SectionId::Done))
        .flat_map(|section| section.threads.iter().map(|thread| thread.id.as_str()))
        .collect();
    let order = prefs::visible_order(sections, is_open, Some(current));
    let index = order.iter().position(|id| id == current)?;
    order[index + 1..]
        .iter()
        .chain(&order[..index])
        .find(|id| !parked.contains(id.as_str()))
        .cloned()
}

/// The snooze menu of a thread: the choices at this minute, soonest first with their time, then
/// "Custom…".
pub(super) fn snooze_menu(
    menu: PopoverMenu,
    this: WeakEntity<Sidebar>,
    thread_id: String,
    presets: &[SnoozePreset],
) -> PopoverMenu {
    let shown = snooze_choices(presets, &Local::now());
    let choices: Vec<MenuOption> = shown
        .iter()
        .map(|choice| {
            let (this, thread_id, shown) = (this.clone(), thread_id.clone(), shown.clone());
            let id = choice.id.clone();
            MenuOption::new(
                SharedString::from(format!("snooze-{}", choice.id)),
                choice_label(choice.label),
            )
            .shortcut(choice_time(choice, &Local))
            .on_select(move |window, cx| {
                this.update(cx, |this, cx| {
                    this.pick_snooze(thread_id.clone(), &shown, &id, window, cx)
                })
                .ok();
            })
        })
        .collect();
    let menu = if choices.is_empty() {
        menu
    } else {
        menu.items(choices).separator()
    };
    menu.item(
        MenuOption::new("snooze-custom", m::snooze_custom()).on_select(move |window, cx| {
            this.update(cx, |this, cx| {
                this.open_custom_snooze(thread_id.clone(), window, cx)
            })
            .ok();
        }),
    )
}

impl Sidebar {
    fn pick_snooze(
        &mut self,
        thread_id: String,
        shown: &[SnoozeChoice],
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let presets = &self.workspace.read(cx).settings.general.snooze_presets;
        match pick_snooze_choice(shown, id, presets, &Local::now()) {
            Some(choice) => self.snooze_thread(thread_id, choice.until, window, cx),
            None => show_failure(
                m::snooze_failed(),
                m::snooze_error_time_passed(),
                window,
                cx,
            ),
        }
    }

    /// Snoozes the thread until `until`; the open one gives way to the next thread listed.
    pub(super) fn snooze_thread(
        &mut self,
        thread_id: String,
        until: DateTime<Utc>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open = self.shown_thread.as_deref() == Some(thread_id.as_str());
        let next = open.then(|| {
            let searching = !self.search.read(cx).value().is_empty();
            next_unparked(
                &self.current_sections(cx),
                |section| searching || !self.prefs.collapsed.contains(&section),
                &thread_id,
            )
        });
        let workspace = self.workspace.read(cx);
        let thread = workspace.model.thread(&thread_id);
        let project_id = thread.and_then(|thread| thread.project_id.clone());
        let restore = thread
            .filter(|thread| is_snoozed(&workspace.model, thread, Utc::now()))
            .and_then(|thread| thread.snoozed_until.as_deref())
            .and_then(|until| DateTime::parse_from_rfc3339(until).ok())
            .map(|until| until.with_timezone(&Utc));
        let engine = workspace.engine.clone();
        let command = Command::ThreadSnooze {
            thread_id: thread_id.clone(),
            until: until.to_rfc3339_opts(SecondsFormat::Millis, true),
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { engine.dispatch(command) })
                .await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(()) => {
                    let left = next
                        .is_some_and(|next| this.leave_snoozed(&thread_id, next, project_id, cx));
                    this.offer_undo(thread_id, restore, left, cx);
                }
                Err(error) => show_failure(m::snooze_failed(), error_message(&error), window, cx),
            })
            .ok();
        })
        .detach();
    }

    /// Moves on from the snoozed thread when it is still open; `true` when it did.
    fn leave_snoozed(
        &mut self,
        thread_id: &str,
        next: Option<String>,
        project_id: Option<String>,
        cx: &mut Context<Self>,
    ) -> bool {
        let still_open = self.workspace.read(cx).open_thread_id(cx);
        if still_open.as_deref() != Some(thread_id) {
            return false;
        }
        match next {
            Some(next) => self.open_thread(&next, cx),
            None => self
                .workspace
                .update(cx, |workspace, cx| workspace.start_draft(project_id, cx)),
        }
        true
    }

    /// "Snoozed thread" with Undo for a few seconds.
    fn offer_undo(
        &mut self,
        thread_id: String,
        restore: Option<DateTime<Utc>>,
        reopen: bool,
        cx: &mut Context<Self>,
    ) {
        let this = cx.weak_entity();
        let action = ToastAction {
            label: m::undo().into(),
            shortcut: Some(toast::undo_shortcut(cx)),
            run: std::rc::Rc::new(move |window, cx| {
                this.update(cx, |this, cx| {
                    this.undo_snooze(thread_id.clone(), restore, reopen, window, cx)
                })
                .ok();
            }),
        };
        toast::show(
            Toast {
                icon: IconName::AlarmClock,
                message: m::snoozed_thread_notice().into(),
                action: Some(action),
            },
            cx,
        );
    }

    /// Takes the snooze back and, when snoozing moved on from the thread, opens it again.
    pub(super) fn undo_snooze(
        &mut self,
        thread_id: String,
        restore: Option<DateTime<Utc>>,
        reopen: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let engine = self.workspace.read(cx).engine.clone();
        let command = undo_command(&thread_id, restore, Utc::now());
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move { engine.dispatch(command) })
                .await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(()) if reopen => this.open_thread(&thread_id, cx),
                Ok(()) => {}
                Err(error) => show_failure(m::wake_failed(), error_message(&error), window, cx),
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn wake_thread(
        &mut self,
        thread_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let engine = self.workspace.read(cx).engine.clone();
        cx.spawn_in(window, async move |_, cx| {
            let result = cx
                .background_spawn(async move { engine.dispatch(Command::ThreadWake { thread_id }) })
                .await;
            if let Err(error) = result {
                cx.update(|window, cx| {
                    show_failure(m::wake_failed(), error_message(&error), window, cx)
                })
                .ok();
            }
        })
        .detach();
    }

    pub(super) fn dismiss_woke(&mut self, thread_id: String, cx: &mut Context<Self>) {
        self.dispatch(Command::ThreadWokeDismiss { thread_id }, cx);
    }

    fn open_custom_snooze(
        &mut self,
        thread_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let form = cx.new(|cx| CustomSnoozeForm::new(thread_id, window, cx));
        let subscription = cx.subscribe_in(
            &form,
            window,
            |this, _, event: &CustomSnoozePicked, window, cx| {
                this.custom_snooze = None;
                this.snooze_thread(event.thread_id.clone(), event.until, window, cx);
                cx.notify();
            },
        );
        self.custom_snooze = Some((form, subscription));
        cx.notify();
    }

    pub(super) fn custom_snooze_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (form, _) = self.custom_snooze.as_ref()?;
        let weak = form.downgrade();
        Some(
            Dialog::new("custom-snooze")
                .open(true)
                .size(DialogSize::Custom(px(448.)))
                .close_button(false)
                .on_open_change(cx.listener(|this, change: &OpenChange, _, cx| {
                    if !change.open {
                        this.custom_snooze = None;
                        cx.notify();
                    }
                }))
                .title(m::custom_snooze_title())
                .description(m::custom_snooze_description())
                .child(form.clone())
                .footer(
                    DialogButton::new("custom-snooze-cancel")
                        .label(m::cancel())
                        .variant(DialogButtonVariant::Outline)
                        .close(),
                )
                .footer(
                    DialogButton::new("custom-snooze-submit")
                        .label(m::custom_snooze_submit())
                        .on_click(move |_, _, cx| {
                            weak.update(cx, |form, cx| form.submit(cx)).ok();
                        }),
                )
                .into_any_element(),
        )
    }
}

/// The user picked a valid time in the custom snooze dialog.
pub(super) struct CustomSnoozePicked {
    thread_id: String,
    until: DateTime<Utc>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    At,
    For,
}

/// "Custom…": a day on a calendar and a time, or a length of time from now.
pub(super) struct CustomSnoozeForm {
    thread_id: String,
    mode: Mode,
    /// The first day of the month the calendar shows.
    month: NaiveDate,
    day: NaiveDate,
    time: Entity<InputState>,
    amount: Entity<InputState>,
    unit: SnoozeUnit,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<CustomSnoozePicked> for CustomSnoozeForm {}

impl CustomSnoozeForm {
    fn new(thread_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let today = Local::now().date_naive();
        let day = today.checked_add_days(Days::new(1)).unwrap_or(today);
        let mut field = |placeholder: String, value: String, cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value)
            })
        };
        let time = field(m::custom_snooze_time_placeholder(), "09:00".to_owned(), cx);
        let amount = field(String::new(), "2".to_owned(), cx);
        let subscriptions = [&time, &amount]
            .into_iter()
            .map(|input| {
                cx.subscribe(
                    input,
                    |this: &mut Self, _, event: &InputEvent, cx| match event {
                        InputEvent::Change => {
                            this.error = None;
                            cx.notify();
                        }
                        InputEvent::PressEnter { .. } => this.submit(cx),
                        _ => {}
                    },
                )
            })
            .collect();
        time.update(cx, |time, cx| time.focus(window, cx));
        Self {
            thread_id,
            mode: Mode::At,
            month: month_of(day),
            day,
            time,
            amount,
            unit: SnoozeUnit::Hours,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    fn input(&self, cx: &App) -> CustomSnooze {
        match self.mode {
            Mode::At => CustomSnooze::At {
                date: self.day.format("%Y-%m-%d").to_string(),
                time: self.time.read(cx).value().trim().to_owned(),
            },
            Mode::For => CustomSnooze::For {
                amount: self.amount.read(cx).value().to_string(),
                unit: self.unit,
            },
        }
    }

    pub(super) fn submit(&mut self, cx: &mut Context<Self>) {
        match custom_snooze_until(&self.input(cx), &Local::now()) {
            Some(until) => cx.emit(CustomSnoozePicked {
                thread_id: self.thread_id.clone(),
                until,
            }),
            None => {
                self.error = Some(match self.mode {
                    Mode::At => m::custom_snooze_invalid_date(),
                    Mode::For => m::custom_snooze_invalid_duration(),
                });
                cx.notify();
            }
        }
    }

    pub(super) fn set_mode(&mut self, mode: Mode, cx: &mut Context<Self>) {
        self.mode = mode;
        self.error = None;
        cx.notify();
    }

    /// "Wed, Nov 18 · 9:00 AM": when the date and time picked wake the thread, on the local
    /// clock; `None` while that isn't a time ahead.
    fn resolved(&self, cx: &App) -> Option<String> {
        let input = CustomSnooze::At {
            date: self.day.format("%Y-%m-%d").to_string(),
            time: self.time.read(cx).value().trim().to_owned(),
        };
        let until = custom_snooze_until(&input, &Local::now())?.with_timezone(&Local);
        Some(m::custom_snooze_resolved(
            until.format("%a, %b %-d"),
            until.format("%-I:%M %p"),
        ))
    }

    #[cfg(test)]
    pub(super) fn picked_day(&self) -> NaiveDate {
        self.day
    }

    #[cfg(test)]
    pub(super) fn type_duration(
        &mut self,
        amount: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mode = Mode::For;
        self.amount.update(cx, |field, cx| {
            field.set_value(amount.to_owned(), window, cx)
        });
    }

    #[cfg(test)]
    pub(super) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl Render for CustomSnoozeForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let tab = |mode: Mode, label: String, cx: &mut Context<Self>| {
            Segment::new(
                div()
                    .px_2p5()
                    .py_1()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .child(label),
            )
            .selected(self.mode == mode)
            .on_click(cx.listener(move |this, _, _, cx| this.set_mode(mode, cx)))
        };
        let tabs = Segmented::new("custom-snooze-mode")
            .segment(tab(Mode::At, m::custom_snooze_date_tab(), cx))
            .segment(tab(Mode::For, m::custom_snooze_duration_tab(), cx));
        let field = |label: String, control: AnyElement| {
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().text_xs().font_weight(FontWeight::MEDIUM).child(label))
                .child(control)
        };
        let date = {
            let today = Local::now().date_naive();
            let (pick, show) = (cx.weak_entity(), cx.weak_entity());
            let calendar =
                MonthCalendar::new("custom-snooze-calendar", self.month, self.day, today)
                    .on_pick(move |day, _, cx| {
                        pick.update(cx, |this, cx| {
                            this.day = day;
                            this.error = None;
                            cx.notify();
                        })
                        .ok();
                    })
                    .on_month(move |month, _, cx| {
                        show.update(cx, |this, cx| {
                            this.month = month;
                            cx.notify();
                        })
                        .ok();
                    });
            let time = div()
                .flex()
                .items_center()
                .gap_2()
                .text_sm()
                .child(
                    div()
                        .text_color(colors.muted_foreground)
                        .child(m::custom_snooze_time()),
                )
                .child(div().w(px(80.)).child(Input::new(&self.time)))
                .child(div().flex_1())
                .when_some(self.resolved(cx), |this, resolved| {
                    this.child(
                        div()
                            .debug_selector(|| "custom-snooze-resolved".into())
                            .text_xs()
                            .text_color(colors.muted_foreground)
                            .child(resolved),
                    )
                });
            div().flex().flex_col().gap_3().child(calendar).child(time)
        };
        let duration = {
            let this = cx.weak_entity();
            let unit = Select::new("custom-snooze-unit", unit_label(self.unit))
                .min_width(px(128.))
                .options(UNITS.into_iter().enumerate().map(|(index, unit)| {
                    let this = this.clone();
                    SelectOption::new(
                        ("custom-snooze-unit", index),
                        unit_label(unit),
                        unit == self.unit,
                    )
                    .on_select(move |_, cx| {
                        this.update(cx, |this, cx| {
                            this.unit = unit;
                            this.error = None;
                            cx.notify();
                        })
                        .ok();
                    })
                }));
            div()
                .flex()
                .gap_3()
                .child(field(
                    m::custom_snooze_amount(),
                    Input::new(&self.amount).into_any_element(),
                ))
                .child(field(m::custom_snooze_unit(), unit.into_any_element()))
        };
        let duration_shown = self.mode == Mode::For;
        let fields = div()
            .relative()
            .child(date.when(duration_shown, |this| this.invisible()))
            .when(duration_shown, |this| {
                this.child(div().absolute().top_0().left_0().right_0().child(duration))
            });
        div()
            .debug_selector(|| "custom-snooze-form".into())
            .flex()
            .flex_col()
            .gap_4()
            .child(tabs)
            .child(fields)
            .when_some(self.error.clone(), |this, error| {
                this.child(
                    div()
                        .debug_selector(|| "custom-snooze-error".into())
                        .text_xs()
                        .text_color(colors.destructive)
                        .child(error),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{next_unparked, undo_command};
    use crate::sections::{Section, SectionId};
    use crate::sidebar::prefs::sample_thread;
    use hoshi_core::Command;

    fn section(id: SectionId, threads: &[&str]) -> Section {
        Section {
            id,
            threads: threads.iter().map(|id| sample_thread(id)).collect(),
        }
    }

    #[test]
    fn snoozing_the_open_thread_moves_on_to_the_next_one_still_listed() {
        let sections = [
            section(SectionId::Active, &["a", "b"]),
            section(SectionId::Inactive, &["c"]),
            section(SectionId::Snoozed, &["s"]),
            section(SectionId::Done, &["d"]),
        ];
        let open = |_: SectionId| true;
        assert_eq!(next_unparked(&sections, open, "b"), Some("c".into()));
        assert_eq!(next_unparked(&sections, open, "c"), Some("a".into()));
        let folded = |id: SectionId| id != SectionId::Inactive;
        assert_eq!(next_unparked(&sections, folded, "b"), Some("a".into()));
        assert_eq!(
            next_unparked(&[section(SectionId::Active, &["a"])], open, "a"),
            None
        );
        assert_eq!(next_unparked(&sections, open, "gone"), None);
    }

    #[test]
    fn undo_puts_back_a_snooze_still_ahead_or_wakes_the_thread() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-15T12:00:00Z")
            .unwrap()
            .to_utc();
        let later = now + chrono::Duration::hours(2);
        assert_eq!(
            undo_command("t", Some(later), now),
            Command::ThreadSnooze {
                thread_id: "t".into(),
                until: "2026-11-15T14:00:00.000Z".into(),
            }
        );
        let wake = Command::ThreadWake {
            thread_id: "t".into(),
        };
        assert_eq!(undo_command("t", None, now), wake);
        assert_eq!(
            undo_command("t", Some(now - chrono::Duration::minutes(1)), now),
            wake
        );
    }
}
