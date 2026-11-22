//! Snoozing a thread: it leaves its section until a time, unless it needs the user sooner.
//! Nothing wakes it on a timer: whether it is snoozed is worked out from the time each time.

use crate::{CommandError, Event, ItemContent, ReadModel, Thread, ThreadItem, ThreadStatus};
use chrono::{
    DateTime, Datelike, Days, NaiveDate, NaiveTime, SecondsFormat, TimeDelta, TimeZone, Timelike,
    Utc, Weekday,
};
use serde::{Deserialize, Serialize};

/// The most choices a snooze menu offers.
pub const MAX_SNOOZE_PRESETS: usize = 8;
/// The furthest a menu choice snoozes for, in minutes: 30 days.
pub const MAX_SNOOZE_MINUTES: u32 = 30 * 24 * 60;

const MINUTE_MS: i64 = 60_000;
const HOUR_MS: i64 = 60 * MINUTE_MS;
const DAY_MS: i64 = 24 * HOUR_MS;

/// A choice of the snooze menu, as the settings keep it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SnoozePreset {
    /// This many minutes after it is picked.
    After { minutes: u32 },
    /// At `time`, `HH:MM` on the local clock, on `day`.
    At { day: SnoozeDay, time: String },
}

/// A weekday is the next one after today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SnoozeDay {
    Today,
    Tomorrow,
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl SnoozeDay {
    fn name(self) -> &'static str {
        match self {
            Self::Today => "today",
            Self::Tomorrow => "tomorrow",
            Self::Monday => "monday",
            Self::Tuesday => "tuesday",
            Self::Wednesday => "wednesday",
            Self::Thursday => "thursday",
            Self::Friday => "friday",
            Self::Saturday => "saturday",
            Self::Sunday => "sunday",
        }
    }

    fn weekday(self) -> Option<Weekday> {
        match self {
            Self::Today | Self::Tomorrow => None,
            Self::Monday => Some(Weekday::Mon),
            Self::Tuesday => Some(Weekday::Tue),
            Self::Wednesday => Some(Weekday::Wed),
            Self::Thursday => Some(Weekday::Thu),
            Self::Friday => Some(Weekday::Fri),
            Self::Saturday => Some(Weekday::Sat),
            Self::Sunday => Some(Weekday::Sun),
        }
    }
}

/// In 1 hour, in 3 hours, today at 18:00, tomorrow at 09:00 and Monday at 09:00.
pub fn default_snooze_presets() -> Vec<SnoozePreset> {
    let at = |day, time: &str| SnoozePreset::At {
        day,
        time: time.to_owned(),
    };
    vec![
        SnoozePreset::After { minutes: 60 },
        SnoozePreset::After { minutes: 180 },
        at(SnoozeDay::Today, "18:00"),
        at(SnoozeDay::Tomorrow, "09:00"),
        at(SnoozeDay::Monday, "09:00"),
    ]
}

impl SnoozePreset {
    /// The same whatever time it resolves to, such as `after:60` or `at:today:18:00`.
    pub fn id(&self) -> String {
        match self {
            Self::After { minutes } => format!("after:{minutes}"),
            Self::At { day, time } => format!("at:{}:{time}", day.name()),
        }
    }

    /// From a minute to 30 days, or a real time of day.
    pub fn is_valid(&self) -> bool {
        match self {
            Self::After { minutes } => (1..=MAX_SNOOZE_MINUTES).contains(minutes),
            Self::At { time, .. } => clock_time(time).is_some(),
        }
    }
}

/// A snooze menu row: what its label says, as a code the UI words, and when it wakes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnoozeChoice {
    /// Its preset's [`SnoozePreset::id`].
    pub id: String,
    pub label: SnoozeLabel,
    pub until: DateTime<Utc>,
    /// It wakes on another day than today, which the label doesn't name: its time shows with
    /// the short weekday.
    pub shows_weekday: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnoozeLabel {
    /// "In 1 hour".
    In(SnoozeAmount),
    /// Today, before noon.
    ThisMorning,
    /// Today, before 17:00.
    ThisAfternoon,
    ThisEvening,
    Tomorrow,
    /// The Monday choice, unless Monday is tomorrow.
    NextWeek,
    /// The weekday a week ahead, as "Next Friday".
    NextWeekday(Weekday),
    Weekday(Weekday),
}

/// A length of time in the largest unit that divides it evenly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnoozeAmount {
    pub count: u32,
    pub unit: SnoozeUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnoozeUnit {
    Minutes,
    Hours,
    Days,
}

impl SnoozeUnit {
    fn millis(self) -> i64 {
        match self {
            Self::Minutes => MINUTE_MS,
            Self::Hours => HOUR_MS,
            Self::Days => DAY_MS,
        }
    }
}

fn amount(minutes: u32) -> SnoozeAmount {
    let (count, unit) = if minutes.is_multiple_of(24 * 60) {
        (minutes / (24 * 60), SnoozeUnit::Days)
    } else if minutes.is_multiple_of(60) {
        (minutes / 60, SnoozeUnit::Hours)
    } else {
        (minutes, SnoozeUnit::Minutes)
    };
    SnoozeAmount { count, unit }
}

/// `HH:MM`, both two digits.
fn clock_time(text: &str) -> Option<NaiveTime> {
    let bytes = text.as_bytes();
    let shaped = bytes.len() == 5
        && bytes.iter().enumerate().all(|(i, byte)| match i {
            2 => *byte == b':',
            _ => byte.is_ascii_digit(),
        });
    if !shaped {
        return None;
    }
    NaiveTime::from_hms_opt(text[..2].parse().ok()?, text[3..].parse().ok()?, 0)
}

/// `YYYY-MM-DD` of a day that exists.
fn calendar_date(text: &str) -> Option<NaiveDate> {
    let bytes = text.as_bytes();
    let shaped = bytes.len() == 10
        && bytes.iter().enumerate().all(|(i, byte)| match i {
            4 | 7 => *byte == b'-',
            _ => byte.is_ascii_digit(),
        });
    shaped
        .then(|| NaiveDate::parse_from_str(text, "%Y-%m-%d").ok())
        .flatten()
}

/// The preset as a menu row at `now`, on `now`'s clock; `None` for a time today that passed.
pub fn snooze_choice<Tz: TimeZone>(
    preset: &SnoozePreset,
    now: &DateTime<Tz>,
) -> Option<SnoozeChoice> {
    let today = now.date_naive();
    let (until, label, days_ahead) = match preset {
        SnoozePreset::After { minutes } => {
            let until = now.clone() + TimeDelta::minutes((*minutes).into());
            (until, SnoozeLabel::In(amount(*minutes)), None)
        }
        SnoozePreset::At { day, time } => {
            let time = clock_time(time)?;
            let days_ahead = match day.weekday() {
                None if *day == SnoozeDay::Today => 0,
                None => 1,
                Some(weekday) => {
                    let ahead = (weekday.num_days_from_monday() + 7
                        - today.weekday().num_days_from_monday())
                        % 7;
                    if ahead == 0 { 7 } else { ahead }
                }
            };
            let date = today.checked_add_days(Days::new(days_ahead.into()))?;
            let naive = date.and_time(time);
            let zone = now.timezone();
            let until = zone.from_local_datetime(&naive).earliest().or_else(|| {
                zone.from_local_datetime(&(naive + TimeDelta::hours(1)))
                    .earliest()
            })?;
            if until <= *now {
                return None;
            }
            let label = match (days_ahead, day.weekday()) {
                (0, _) if time.hour() < 12 => SnoozeLabel::ThisMorning,
                (0, _) if time.hour() < 17 => SnoozeLabel::ThisAfternoon,
                (0, _) => SnoozeLabel::ThisEvening,
                (1, _) => SnoozeLabel::Tomorrow,
                (_, Some(Weekday::Mon)) => SnoozeLabel::NextWeek,
                (7, Some(weekday)) => SnoozeLabel::NextWeekday(weekday),
                (_, Some(weekday)) => SnoozeLabel::Weekday(weekday),
                (_, None) => SnoozeLabel::Tomorrow,
            };
            (until, label, Some(days_ahead))
        }
    };
    let names_day = days_ahead.is_some() && label != SnoozeLabel::NextWeek;
    Some(SnoozeChoice {
        id: preset.id(),
        label,
        shows_weekday: !names_day && until.date_naive() != today,
        until: until.with_timezone(&Utc),
    })
}

/// The snooze menu at `now`: the valid presets among the first [`MAX_SNOOZE_PRESETS`],
/// soonest first, one per wake minute, the one listed first in the settings kept.
pub fn snooze_choices<Tz: TimeZone>(
    presets: &[SnoozePreset],
    now: &DateTime<Tz>,
) -> Vec<SnoozeChoice> {
    let minute = |choice: &SnoozeChoice| choice.until.timestamp_millis().div_euclid(MINUTE_MS);
    let mut choices: Vec<SnoozeChoice> = presets
        .iter()
        .filter(|preset| preset.is_valid())
        .take(MAX_SNOOZE_PRESETS)
        .filter_map(|preset| snooze_choice(preset, now))
        .collect();
    choices.sort_by_key(minute);
    choices.dedup_by_key(|choice| minute(choice));
    choices
}

/// The choice `id` of the menu `shown`, as picked at `now`: a duration starts now, a set time
/// keeps the time shown. `None` once the time shown passed: the user picks another.
pub fn pick_snooze_choice<Tz: TimeZone>(
    shown: &[SnoozeChoice],
    id: &str,
    presets: &[SnoozePreset],
    now: &DateTime<Tz>,
) -> Option<SnoozeChoice> {
    let restarted = presets
        .iter()
        .find(|preset| matches!(preset, SnoozePreset::After { .. }) && preset.id() == id)
        .and_then(|preset| snooze_choice(preset, now));
    restarted.or_else(|| {
        shown
            .iter()
            .find(|choice| choice.id == id && choice.until > now.with_timezone(&Utc))
            .cloned()
    })
}

/// A time the user typed for a snooze.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustomSnooze {
    /// `date` as `YYYY-MM-DD` and `time` as `HH:MM`, on the local clock.
    At { date: String, time: String },
    /// `amount`, as typed, of `unit` from now.
    For { amount: String, unit: SnoozeUnit },
}

/// When the typed snooze wakes, on `now`'s clock; `None` when it isn't a time after `now`
/// (a day or a local time that doesn't exist included) or not a positive amount.
pub fn custom_snooze_until<Tz: TimeZone>(
    input: &CustomSnooze,
    now: &DateTime<Tz>,
) -> Option<DateTime<Utc>> {
    let now = now.clone();
    let until = match input {
        CustomSnooze::For { amount, unit } => {
            let amount: f64 = amount.trim().parse().ok()?;
            if !amount.is_finite() || amount <= 0.0 {
                return None;
            }
            let millis = amount * unit.millis() as f64;
            if millis >= i64::MAX as f64 {
                return None;
            }
            now.with_timezone(&Utc)
                .checked_add_signed(TimeDelta::try_milliseconds(millis as i64)?)?
        }
        CustomSnooze::At { date, time } => {
            let naive = calendar_date(date)?.and_time(clock_time(time)?);
            now.timezone()
                .from_local_datetime(&naive)
                .earliest()?
                .with_timezone(&Utc)
        }
    };
    (until > now.with_timezone(&Utc)).then_some(until)
}

/// How long a snooze has left, rounded up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnoozeLeft {
    Now,
    Minutes(u32),
    Hours(u32),
    Days(u32),
}

pub fn snooze_left(until: &str, now: DateTime<Utc>) -> SnoozeLeft {
    let Some(until) = instant(until) else {
        return SnoozeLeft::Now;
    };
    let left = (until - now).num_milliseconds();
    if left <= 0 {
        return SnoozeLeft::Now;
    }
    let up = |unit: i64| u32::try_from((left + unit - 1) / unit).unwrap_or(u32::MAX);
    let (minutes, hours) = (up(MINUTE_MS), up(HOUR_MS));
    if minutes < 60 {
        SnoozeLeft::Minutes(minutes)
    } else if hours < 24 {
        SnoozeLeft::Hours(hours)
    } else {
        SnoozeLeft::Days(up(DAY_MS))
    }
}

fn instant(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The thread's wake time is still ahead, whatever woke it since.
pub fn snooze_pending(thread: &Thread, now: DateTime<Utc>) -> bool {
    thread
        .snoozed_until
        .as_deref()
        .and_then(instant)
        .is_some_and(|until| until > now)
}

/// Snoozed now: its wake time is ahead and nothing since its snooze asks for the user. Its
/// section is Snoozed, it notifies of nothing, and hoshi doesn't go on with it by itself.
pub fn is_snoozed(model: &ReadModel, thread: &Thread, now: DateTime<Utc>) -> bool {
    !thread.archived && snooze_pending(thread, now) && raised_hand(model, thread).is_none()
}

/// Since its snooze, the thread asks for the user, which wakes it early: an approval or another
/// request that isn't a question, a question asked after the snooze, a turn that failed after
/// it, or a request of a task it delegated. A turn that just completed never wakes it.
pub fn raised_hand_while_snoozed(model: &ReadModel, thread: &Thread) -> bool {
    raised_hand(model, thread).is_some()
}

/// When the thread woke from its snooze, early or on time; `None` while snoozed or without a
/// snooze to wake from.
pub fn woke_at(model: &ReadModel, thread: &Thread, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let until = instant(thread.snoozed_until.as_deref()?)?;
    let Some(raised) = raised_hand(model, thread) else {
        return (until <= now).then_some(until);
    };
    let snoozed_at = thread.snoozed_at.as_deref().and_then(instant);
    let question = raised
        .question_at
        .filter(|asked| snoozed_at.is_some_and(|snoozed| *asked > snoozed));
    match (raised.failed_at, question) {
        (Some(failed), asked) if asked.is_none_or(|asked| failed >= asked) => Some(failed),
        (_, Some(asked)) => Some(asked),
        _ => raised.requested_at.or(snoozed_at),
    }
}

/// The Woke indicator: the thread woke after the user last opened it and after its latest
/// turn ended, which the user saw finish.
pub fn shows_woke(model: &ReadModel, thread: &Thread, now: DateTime<Utc>) -> bool {
    if thread.archived {
        return false;
    }
    let Some(woke) = woke_at(model, thread, now) else {
        return false;
    };
    [&thread.last_visited_at, &thread.last_turn_ended_at]
        .into_iter()
        .filter_map(|seen| seen.as_deref().and_then(instant))
        .all(|seen| seen < woke)
}

/// When the thread's inactivity counts from: its last activity, or its wake if later, so a
/// thread back from a long snooze doesn't count as inactive.
pub fn active_since(thread: &Thread, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let passed = thread
        .snoozed_until
        .as_deref()
        .and_then(instant)
        .filter(|until| *until <= now);
    [
        instant(&thread.updated_at),
        thread.last_snooze_wake_at.as_deref().and_then(instant),
        passed,
    ]
    .into_iter()
    .flatten()
    .max()
}

/// What, since its snooze, asks for the user, with when as far as known.
struct RaisedHand {
    failed_at: Option<DateTime<Utc>>,
    question_at: Option<DateTime<Utc>>,
    requested_at: Option<DateTime<Utc>>,
}

fn is_question(item: &ThreadItem) -> bool {
    matches!(item.content, ItemContent::Questions { .. })
}

/// Requests waiting in the tasks the thread delegated, however deep.
fn delegated_requests<'a>(model: &'a ReadModel, thread: &Thread) -> Vec<&'a ThreadItem> {
    let mut parents = vec![thread.id.clone()];
    let mut requests = Vec::new();
    let mut seen = 0;
    while let Some(parent) = parents.pop() {
        seen += 1;
        if seen > model.threads().count() {
            break;
        }
        for child in model
            .threads()
            .filter(|child| child.parent_thread_id.as_deref() == Some(parent.as_str()))
        {
            requests.extend(model.pending_requests_within(&child.id));
            parents.push(child.id.clone());
        }
    }
    requests
}

fn raised_hand(model: &ReadModel, thread: &Thread) -> Option<RaisedHand> {
    let snoozed_at = thread.snoozed_at.as_deref().and_then(instant);
    let mut requested: Option<Option<DateTime<Utc>>> = None;
    let mut asked: Option<Option<DateTime<Utc>>> = None;
    for item in model.pending_requests_within(&thread.id) {
        let at = instant(&item.created_at);
        if !is_question(item) {
            requested = Some(requested.flatten().max(at));
        } else if snoozed_at.zip(at).is_none_or(|(snoozed, at)| at > snoozed) {
            asked = Some(asked.flatten().max(at));
        }
    }
    for item in delegated_requests(model, thread) {
        requested = Some(requested.flatten().max(instant(&item.created_at)));
    }
    let failed_at = (thread.status == ThreadStatus::Error)
        .then(|| thread.last_turn_ended_at.as_deref().and_then(instant));
    let failed_fresh = failed_at.is_some_and(|failed| {
        snoozed_at.is_none() || failed.zip(snoozed_at).is_some_and(|(at, s)| at > s)
    });
    if requested.is_none() && asked.is_none() && !failed_fresh {
        return None;
    }
    Some(RaisedHand {
        failed_at: failed_at.flatten().filter(|_| failed_fresh),
        question_at: asked.flatten(),
        requested_at: requested.flatten(),
    })
}

/// An approval or another request that isn't a question waits for the user, in the thread or
/// a task it delegated: snoozing would hide what it needs now.
fn waits_for_user(model: &ReadModel, thread: &Thread) -> bool {
    model
        .pending_requests_within(&thread.id)
        .any(|item| !is_question(item))
        || !delegated_requests(model, thread).is_empty()
}

pub(crate) fn snooze(
    model: &ReadModel,
    thread_id: String,
    until: &str,
    now: &str,
) -> Result<Vec<Event>, CommandError> {
    let Some(thread) = model.thread(&thread_id) else {
        return Err(CommandError::ThreadNotFound { thread_id });
    };
    let Some(until) = instant(until).filter(|until| instant(now).is_some_and(|now| *until > now))
    else {
        return Err(CommandError::SnoozeTimePassed { thread_id });
    };
    if thread.archived {
        return Err(CommandError::ThreadArchived { thread_id });
    }
    if waits_for_user(model, thread) {
        return Err(CommandError::ThreadWaitingForUser { thread_id });
    }
    Ok(vec![Event::ThreadSnoozed {
        thread_id,
        until: stamp(until),
        at: now.to_owned(),
    }])
}

/// Snoozes the thread until its usage limit resets at `resets_at`, unless nothing would hide
/// it: the reset passed, or the thread is archived or waits for the user.
pub fn snooze_until_reset(
    model: &ReadModel,
    thread_id: &str,
    resets_at: &str,
    now: &str,
) -> Vec<Event> {
    snooze(model, thread_id.to_owned(), resets_at, now).unwrap_or_default()
}

/// Ends the thread's snooze, when it has one; `woke_at` is set when the user woke it.
pub(crate) fn unsnoozed(thread: &Thread, woke_at: Option<&str>) -> Option<Event> {
    thread
        .snoozed_until
        .as_ref()
        .map(|_| Event::ThreadUnsnoozed {
            thread_id: thread.id.clone(),
            woke_at: woke_at.map(str::to_owned),
        })
}

/// A visit, kept only when it hides the Woke indicator: no other visit changes what it shows.
pub(crate) fn visited(model: &ReadModel, thread: &Thread, now: &str) -> Option<Event> {
    instant(now)
        .is_some_and(|at| shows_woke(model, thread, at))
        .then(|| Event::ThreadVisited {
            thread_id: thread.id.clone(),
            at: now.to_owned(),
        })
}

/// A visit at the time the thread woke, which hides its Woke indicator.
pub(crate) fn woke_dismissed(model: &ReadModel, thread: &Thread, now: &str) -> Option<Event> {
    let woke = woke_at(model, thread, instant(now)?)?;
    let seen = thread.last_visited_at.as_deref().and_then(instant);
    seen.is_none_or(|seen| seen < woke)
        .then(|| Event::ThreadVisited {
            thread_id: thread.id.clone(),
            at: stamp(woke),
        })
}

#[cfg(test)]
mod tests;
