//! Settings → Threads → Snooze options: the presets of every snooze menu, one editable row each,
//! with how the row shows in the menu now or why it doesn't.

use crate::m;
use crate::snooze::{choice_label, choice_time, weekday_name};
use crate::state::Workspace;
use chrono::{DateTime, Local, TimeZone, Weekday};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use hoshi_core::{
    MAX_SNOOZE_MINUTES, MAX_SNOOZE_PRESETS, SnoozeChoice, SnoozeDay, SnoozeLabel, SnoozePreset,
    SnoozeUnit, default_snooze_presets, snooze_choice, snooze_choices,
};
use hoshi_ui::prelude::*;
use std::collections::HashSet;
use std::time::Duration;

const UNITS: [SnoozeUnit; 3] = [SnoozeUnit::Minutes, SnoozeUnit::Hours, SnoozeUnit::Days];
const DAYS: [SnoozeDay; 9] = [
    SnoozeDay::Today,
    SnoozeDay::Tomorrow,
    SnoozeDay::Monday,
    SnoozeDay::Tuesday,
    SnoozeDay::Wednesday,
    SnoozeDay::Thursday,
    SnoozeDay::Friday,
    SnoozeDay::Saturday,
    SnoozeDay::Sunday,
];

fn unit_minutes(unit: SnoozeUnit) -> u32 {
    match unit {
        SnoozeUnit::Minutes => 1,
        SnoozeUnit::Hours => 60,
        SnoozeUnit::Days => 24 * 60,
    }
}

fn unit_name(unit: SnoozeUnit, count: u32) -> String {
    match (unit, count == 1) {
        (SnoozeUnit::Minutes, true) => m::snooze_option_unit_minute(),
        (SnoozeUnit::Minutes, false) => m::snooze_option_unit_minutes(),
        (SnoozeUnit::Hours, true) => m::snooze_option_unit_hour(),
        (SnoozeUnit::Hours, false) => m::snooze_option_unit_hours(),
        (SnoozeUnit::Days, true) => m::snooze_option_unit_day(),
        (SnoozeUnit::Days, false) => m::snooze_option_unit_days(),
    }
}

fn day_name(day: SnoozeDay) -> String {
    match day {
        SnoozeDay::Today => m::snooze_day_today(),
        SnoozeDay::Tomorrow => m::snooze_day_tomorrow(),
        SnoozeDay::Monday => weekday_name(Weekday::Mon),
        SnoozeDay::Tuesday => weekday_name(Weekday::Tue),
        SnoozeDay::Wednesday => weekday_name(Weekday::Wed),
        SnoozeDay::Thursday => weekday_name(Weekday::Thu),
        SnoozeDay::Friday => weekday_name(Weekday::Fri),
        SnoozeDay::Saturday => weekday_name(Weekday::Sat),
        SnoozeDay::Sunday => weekday_name(Weekday::Sun),
    }
}

/// `minutes` in the largest unit that divides it evenly.
pub(super) fn split_minutes(minutes: u32) -> (u32, SnoozeUnit) {
    UNITS
        .into_iter()
        .rev()
        .find(|unit| minutes.is_multiple_of(unit_minutes(*unit)))
        .map_or((minutes, SnoozeUnit::Minutes), |unit| {
            (minutes / unit_minutes(unit), unit)
        })
}

/// What a row's typed value is wrong about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RowError {
    /// Not a whole number above 0.
    Amount,
    /// More than 30 days.
    TooLong,
    /// Not a time as `HH:MM`.
    Time,
}

fn error_text(error: RowError) -> String {
    match error {
        RowError::Amount => m::snooze_option_invalid_amount(),
        RowError::TooLong => m::snooze_option_too_long(),
        RowError::Time => m::snooze_option_invalid_time(),
    }
}

/// The minutes of `text` `unit`s, as typed in an "After" row.
pub(super) fn amount_minutes(text: &str, unit: SnoozeUnit) -> Result<u32, RowError> {
    let amount: u32 = text
        .trim()
        .parse()
        .ok()
        .filter(|amount| *amount > 0)
        .ok_or(RowError::Amount)?;
    amount
        .checked_mul(unit_minutes(unit))
        .filter(|minutes| *minutes <= MAX_SNOOZE_MINUTES)
        .ok_or(RowError::TooLong)
}

/// The time typed in an "At" row, as kept.
pub(super) fn clock_time(text: &str) -> Result<String, RowError> {
    let time = text.trim().to_owned();
    let preset = SnoozePreset::At {
        day: SnoozeDay::Today,
        time: time.clone(),
    };
    preset.is_valid().then_some(time).ok_or(RowError::Time)
}

/// How a row shows in the menu now, or why it doesn't.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Hint {
    Menu(SnoozeChoice),
    /// Its value can't be used.
    Invalid,
    /// A time today that passed.
    Passed,
    /// The same choice as a row above it.
    Duplicate,
    /// Another choice above it wakes at the same minute.
    SameTime,
}

/// Each row's [`Hint`] at `now`, comparing it only with the rows above it, as the menu keeps the
/// first of a repeat.
pub(super) fn hints<Tz: TimeZone>(presets: &[SnoozePreset], now: &DateTime<Tz>) -> Vec<Hint> {
    let mut ids = HashSet::new();
    let mut minutes = HashSet::new();
    presets
        .iter()
        .map(|preset| {
            if !preset.is_valid() {
                return Hint::Invalid;
            }
            let Some(choice) = snooze_choice(preset, now) else {
                return Hint::Passed;
            };
            if !ids.insert(choice.id.clone()) {
                return Hint::Duplicate;
            }
            if !minutes.insert(choice.until.timestamp_millis().div_euclid(60_000)) {
                return Hint::SameTime;
            }
            Hint::Menu(choice)
        })
        .collect()
}

/// The row "Add option" adds: the first of the usual choices the list lacks.
pub(super) fn next_option(presets: &[SnoozePreset]) -> SnoozePreset {
    let used: HashSet<String> = presets.iter().map(SnoozePreset::id).collect();
    default_snooze_presets()
        .into_iter()
        .chain([
            SnoozePreset::After { minutes: 30 },
            SnoozePreset::At {
                day: SnoozeDay::Saturday,
                time: "09:00".into(),
            },
            SnoozePreset::After { minutes: 24 * 60 },
        ])
        .find(|preset| !used.contains(&preset.id()))
        .unwrap_or(SnoozePreset::After { minutes: 60 })
}

/// The same choices as the defaults, in any order.
fn are_defaults(presets: &[SnoozePreset]) -> bool {
    let ids = |presets: &[SnoozePreset]| {
        let mut ids: Vec<String> = presets.iter().map(SnoozePreset::id).collect();
        ids.sort();
        ids
    };
    ids(presets) == ids(&default_snooze_presets())
}

fn hint_text(hint: &Hint) -> String {
    match hint {
        Hint::Menu(choice) => match choice.label {
            SnoozeLabel::In(_) => choice_label(choice.label),
            label => m::snooze_option_hint(choice_label(label), choice_time(choice, &Local)),
        },
        Hint::Invalid => String::new(),
        Hint::Passed => m::snooze_option_passed(),
        Hint::Duplicate => m::snooze_option_duplicate(),
        Hint::SameTime => m::snooze_option_same_time(),
    }
}

/// Whether a row's hint is a problem to fix, not how the menu handles the row on its own.
fn is_problem(hint: &Hint) -> bool {
    matches!(hint, Hint::Duplicate)
}

/// The list changed: the settings keep it.
pub(super) struct SnoozeOptionsChanged(pub Vec<SnoozePreset>);

struct Row {
    key: usize,
    amount: Entity<InputState>,
    unit: SnoozeUnit,
    time: Entity<InputState>,
    error: Option<RowError>,
}

pub(super) struct SnoozeOptions {
    /// The list as the rows read, kept once a field commits or a choice changes.
    presets: Vec<SnoozePreset>,
    /// The list as last kept.
    saved: Vec<SnoozePreset>,
    rows: Vec<Row>,
    next_key: usize,
    _subscriptions: Vec<Subscription>,
    _tick: Task<()>,
}

impl EventEmitter<SnoozeOptionsChanged> for SnoozeOptions {}

impl SnoozeOptions {
    pub(super) fn new(
        workspace: &Entity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let observe = cx.observe_in(workspace, window, |this, workspace, window, cx| {
            let saved = workspace.read(cx).settings.general.snooze_presets.clone();
            if saved != this.saved {
                this.show(saved, window, cx);
            }
        });
        let tick = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(60))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        let mut options = Self {
            presets: Vec::new(),
            saved: Vec::new(),
            rows: Vec::new(),
            next_key: 0,
            _subscriptions: vec![observe],
            _tick: tick,
        };
        let saved = workspace.read(cx).settings.general.snooze_presets.clone();
        options.show(saved, window, cx);
        options
    }

    /// Shows `presets` in fresh rows.
    fn show(&mut self, presets: Vec<SnoozePreset>, window: &mut Window, cx: &mut Context<Self>) {
        self.rows = presets
            .iter()
            .map(|preset| self.row(preset, window, cx))
            .collect();
        self.saved = presets.clone();
        self.presets = presets;
        cx.notify();
    }

    fn row(&mut self, preset: &SnoozePreset, window: &mut Window, cx: &mut Context<Self>) -> Row {
        let key = self.next_key;
        self.next_key += 1;
        let (amount, unit, time) = match preset {
            SnoozePreset::After { minutes } => {
                let (amount, unit) = split_minutes(*minutes);
                (amount.to_string(), unit, "09:00".to_owned())
            }
            SnoozePreset::At { time, .. } => ("1".to_owned(), SnoozeUnit::Hours, time.clone()),
        };
        let mut field = |value: String, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).default_value(value))
        };
        let (amount, time) = (field(amount, cx), field(time, cx));
        for input in [&amount, &time] {
            cx.subscribe(
                input,
                move |this: &mut Self, _, event: &InputEvent, cx| match event {
                    InputEvent::Change => this.typed(key, cx),
                    InputEvent::Blur | InputEvent::PressEnter { .. } => this.save(cx),
                    _ => {}
                },
            )
            .detach();
        }
        Row {
            key,
            amount,
            unit,
            time,
            error: None,
        }
    }

    fn index(&self, key: usize) -> Option<usize> {
        self.rows.iter().position(|row| row.key == key)
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.presets != self.saved {
            self.saved = self.presets.clone();
            cx.emit(SnoozeOptionsChanged(self.presets.clone()));
        }
        cx.notify();
    }

    /// The row's preset as its fields read now, or what is wrong with them.
    fn read_row(&self, index: usize, cx: &App) -> Result<SnoozePreset, RowError> {
        let row = &self.rows[index];
        match &self.presets[index] {
            SnoozePreset::After { .. } => amount_minutes(&row.amount.read(cx).value(), row.unit)
                .map(|minutes| SnoozePreset::After { minutes }),
            SnoozePreset::At { day, .. } => clock_time(&row.time.read(cx).value())
                .map(|time| SnoozePreset::At { day: *day, time }),
        }
    }

    /// Takes the row as its fields read, when they are right.
    fn apply(&mut self, index: usize, cx: &mut Context<Self>) {
        match self.read_row(index, cx) {
            Ok(preset) => {
                self.rows[index].error = None;
                self.presets[index] = preset;
            }
            Err(error) => self.rows[index].error = Some(error),
        }
        cx.notify();
    }

    fn typed(&mut self, key: usize, cx: &mut Context<Self>) {
        if let Some(index) = self.index(key) {
            self.apply(index, cx);
        }
    }

    fn set_kind(&mut self, key: usize, after: bool, cx: &mut Context<Self>) {
        let Some(index) = self.index(key) else {
            return;
        };
        let is_after = matches!(self.presets[index], SnoozePreset::After { .. });
        if is_after == after {
            return;
        }
        self.presets[index] = if after {
            SnoozePreset::After { minutes: 60 }
        } else {
            SnoozePreset::At {
                day: SnoozeDay::Tomorrow,
                time: "09:00".into(),
            }
        };
        self.rows[index].error = None;
        match self.read_row(index, cx) {
            Ok(preset) => self.presets[index] = preset,
            Err(error) => self.rows[index].error = Some(error),
        }
        self.save(cx);
    }

    fn set_unit(&mut self, key: usize, unit: SnoozeUnit, cx: &mut Context<Self>) {
        if let Some(index) = self.index(key) {
            self.rows[index].unit = unit;
            self.apply(index, cx);
            self.save(cx);
        }
    }

    fn set_day(&mut self, key: usize, day: SnoozeDay, cx: &mut Context<Self>) {
        let Some(index) = self.index(key) else {
            return;
        };
        if let SnoozePreset::At { day: kept, .. } = &mut self.presets[index] {
            *kept = day;
            self.save(cx);
        }
    }

    fn remove(&mut self, key: usize, cx: &mut Context<Self>) {
        if let Some(index) = self.index(key) {
            self.rows.remove(index);
            self.presets.remove(index);
            self.save(cx);
        }
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.presets.len() >= MAX_SNOOZE_PRESETS {
            return;
        }
        let preset = next_option(&self.presets);
        let row = self.row(&preset, window, cx);
        self.rows.push(row);
        self.presets.push(preset);
        self.save(cx);
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let saved = std::mem::take(&mut self.saved);
        self.show(default_snooze_presets(), window, cx);
        self.saved = saved;
        self.save(cx);
    }

    #[cfg(test)]
    pub(super) fn amount_field(&self, index: usize) -> Entity<InputState> {
        self.rows[index].amount.clone()
    }

    #[cfg(test)]
    pub(super) fn errors(&self) -> Vec<Option<RowError>> {
        self.rows.iter().map(|row| row.error).collect()
    }

    fn render_row(
        &self,
        index: usize,
        hint: &Hint,
        colors: &Colors,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let row = &self.rows[index];
        let key = row.key;
        let this = cx.weak_entity();
        let preset = &self.presets[index];
        let after = matches!(preset, SnoozePreset::After { .. });
        let kind_option = |label: String, option: bool| {
            let this = this.clone();
            SelectOption::new(
                SharedString::from(format!("snooze-option-{key}-kind-{option}")),
                label,
                after == option,
            )
            .on_select(move |_, cx| {
                this.update(cx, |this, cx| this.set_kind(key, option, cx))
                    .ok();
            })
        };
        let kind = Select::new(
            SharedString::from(format!("snooze-option-{key}-kind")),
            if after {
                m::snooze_option_after()
            } else {
                m::snooze_option_at()
            },
        )
        .min_width(px(88.))
        .option(kind_option(m::snooze_option_after(), true))
        .option(kind_option(m::snooze_option_at(), false));
        let fields = match preset {
            SnoozePreset::After { minutes } => {
                let count = split_minutes(*minutes).0;
                let unit = Select::new(
                    SharedString::from(format!("snooze-option-{key}-unit")),
                    unit_name(row.unit, count),
                )
                .min_width(px(104.))
                .options(UNITS.into_iter().map(|unit| {
                    let this = this.clone();
                    SelectOption::new(
                        SharedString::from(format!("snooze-option-{key}-unit-{unit:?}")),
                        unit_name(unit, count),
                        unit == row.unit,
                    )
                    .on_select(move |_, cx| {
                        this.update(cx, |this, cx| this.set_unit(key, unit, cx))
                            .ok();
                    })
                }));
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(64.)).child(text_input(&row.amount)))
                    .child(unit)
            }
            SnoozePreset::At { day, .. } => {
                let chosen = *day;
                let day = Select::new(
                    SharedString::from(format!("snooze-option-{key}-day")),
                    day_name(chosen),
                )
                .min_width(px(128.))
                .options(DAYS.into_iter().map(|day| {
                    let this = this.clone();
                    SelectOption::new(
                        SharedString::from(format!("snooze-option-{key}-day-{day:?}")),
                        day_name(day),
                        day == chosen,
                    )
                    .on_select(move |_, cx| {
                        this.update(cx, |this, cx| this.set_day(key, day, cx)).ok();
                    })
                }));
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(day)
                    .child(div().w(px(80.)).child(text_input(&row.time)))
            }
        };
        let (text, color) = match row.error {
            Some(error) => (error_text(error), colors.destructive),
            None if is_problem(hint) => (hint_text(hint), colors.warning),
            None => (hint_text(hint), colors.muted_foreground),
        };
        let selector = format!("snooze-option-{index}-hint");
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .child(kind)
            .child(fields)
            .child(
                div()
                    .debug_selector(move || selector.clone())
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(color)
                    .child(text),
            )
            .child(
                Button::new(SharedString::from(format!("snooze-option-{key}-remove")))
                    .icon(IconName::X)
                    .size(ButtonSize::IconXs)
                    .variant(ButtonVariant::Ghost)
                    .tooltip(Tooltip::new(m::snooze_option_remove()))
                    .on_click(cx.listener(move |this, _, _, cx| this.remove(key, cx))),
            )
            .into_any_element()
    }
}

impl Render for SnoozeOptions {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi().clone();
        let now = Local::now();
        let hints = hints(&self.presets, &now);
        let rows: Vec<AnyElement> = hints
            .iter()
            .enumerate()
            .map(|(index, hint)| self.render_row(index, hint, &colors, cx))
            .collect();
        let only_custom = snooze_choices(&self.presets, &now).is_empty();
        let full = self.presets.len() >= MAX_SNOOZE_PRESETS;
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .child(
                div().debug_selector(|| "snooze-options-add".into()).child(
                    Button::new("snooze-options-add")
                        .icon(IconName::Plus)
                        .label(m::snooze_option_add())
                        .size(ButtonSize::Xs)
                        .variant(ButtonVariant::Ghost)
                        .disabled(full)
                        .on_click(cx.listener(|this, _, window, cx| this.add(window, cx))),
                ),
            )
            .when(full, |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(m::snooze_options_limit(MAX_SNOOZE_PRESETS)),
                )
            })
            .child(div().flex_1())
            .child(
                div()
                    .debug_selector(|| "snooze-options-reset".into())
                    .child(
                        Button::new("snooze-options-reset")
                            .label(m::snooze_options_reset())
                            .size(ButtonSize::Xs)
                            .variant(ButtonVariant::Ghost)
                            .disabled(are_defaults(&self.presets))
                            .on_click(cx.listener(|this, _, window, cx| this.reset(window, cx))),
                    ),
            );
        SettingsGroup::new()
            .title(m::settings_snooze_options())
            .description(m::snooze_options_description())
            .children(rows)
            .when(only_custom, |this| {
                this.child(
                    div()
                        .debug_selector(|| "snooze-options-only-custom".into())
                        .px_4()
                        .py_2()
                        .text_xs()
                        .text_color(colors.muted_foreground)
                        .child(m::snooze_options_only_custom()),
                )
            })
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Hint, RowError, amount_minutes, clock_time, hints, is_problem, next_option, split_minutes,
    };
    use chrono::{DateTime, FixedOffset};
    use hoshi_core::{SnoozeDay, SnoozeLabel, SnoozePreset, SnoozeUnit, default_snooze_presets};

    fn at(text: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(text).unwrap()
    }

    fn at_time(day: SnoozeDay, time: &str) -> SnoozePreset {
        SnoozePreset::At {
            day,
            time: time.into(),
        }
    }

    #[test]
    fn only_a_repeated_option_reads_as_a_problem() {
        assert!(is_problem(&Hint::Duplicate));
        assert!(!is_problem(&Hint::Passed));
        assert!(!is_problem(&Hint::SameTime));
        assert!(!is_problem(&Hint::Invalid));
    }

    #[test]
    fn each_row_says_how_the_menu_shows_it_or_why_it_does_not() {
        let now = at("2026-11-15T19:00:30+00:00");
        let presets = [
            SnoozePreset::After { minutes: 60 },
            at_time(SnoozeDay::Today, "18:00"),
            SnoozePreset::After { minutes: 60 },
            at_time(SnoozeDay::Today, "20:00"),
            at_time(SnoozeDay::Tomorrow, "09:00"),
            at_time(SnoozeDay::Tomorrow, "9:00"),
        ];
        let shown = hints(&presets, &now);
        assert!(
            matches!(&shown[0], Hint::Menu(choice) if matches!(choice.label, SnoozeLabel::In(_)))
        );
        assert_eq!(shown[1], Hint::Passed);
        assert_eq!(shown[2], Hint::Duplicate);
        assert_eq!(shown[3], Hint::SameTime);
        assert!(matches!(&shown[4], Hint::Menu(choice) if choice.label == SnoozeLabel::Tomorrow));
        assert_eq!(shown[5], Hint::Invalid);
    }

    #[test]
    fn durations_show_in_their_largest_whole_unit_and_stop_at_thirty_days() {
        assert_eq!(split_minutes(180), (3, SnoozeUnit::Hours));
        assert_eq!(split_minutes(2 * 24 * 60), (2, SnoozeUnit::Days));
        assert_eq!(split_minutes(90), (90, SnoozeUnit::Minutes));
        assert_eq!(amount_minutes(" 3 ", SnoozeUnit::Hours), Ok(180));
        assert_eq!(amount_minutes("30", SnoozeUnit::Days), Ok(30 * 24 * 60));
        assert_eq!(
            amount_minutes("31", SnoozeUnit::Days),
            Err(RowError::TooLong)
        );
        assert_eq!(
            amount_minutes("0", SnoozeUnit::Hours),
            Err(RowError::Amount)
        );
        assert_eq!(
            amount_minutes("1.5", SnoozeUnit::Hours),
            Err(RowError::Amount)
        );
        assert_eq!(clock_time("18:30"), Ok("18:30".to_owned()));
        assert_eq!(clock_time("25:00"), Err(RowError::Time));
        assert_eq!(clock_time("9:00"), Err(RowError::Time));
    }

    #[test]
    fn adding_offers_a_usual_choice_the_list_lacks() {
        assert_eq!(next_option(&[]), SnoozePreset::After { minutes: 60 });
        assert_eq!(
            next_option(&default_snooze_presets()),
            SnoozePreset::After { minutes: 30 }
        );
    }
}
