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
mod tests {
    use super::*;
    use crate::{AccessMode, Command, Question, RequestResponse, RequestStatus, Snapshot, decide};
    use chrono::FixedOffset;

    const NOW: &str = "2026-11-15T12:00:00.000Z";
    const LATER: &str = "2026-11-15T15:00:00.000Z";

    fn at(text: &str) -> DateTime<Utc> {
        instant(text).unwrap()
    }

    fn thread(id: &str) -> Thread {
        Thread {
            id: id.into(),
            project_id: Some("p".into()),
            title: "thread".into(),
            status: ThreadStatus::Idle,
            branch: None,
            worktree_path: None,
            session_id: Some("s".into()),
            conversation_cursor: None,
            resume_at: None,
            watching_pull_request: false,
            provider: crate::default_provider(),
            model: None,
            effort: None,
            fast_mode: false,
            ultracode: false,
            access_mode: AccessMode::AcceptEdits,
            plan_mode: false,
            active_turn_id: None,
            archived: false,
            pinned: false,
            worktree_released: false,
            worktree_adopted: false,
            parent_thread_id: None,
            task_cancelled: false,
            task_answered: false,
            snoozed_until: None,
            snoozed_at: None,
            last_snooze_wake_at: None,
            last_visited_at: None,
            last_turn_ended_at: None,
            forked_from: None,
            pending_model: None,
            provider_sessions: Vec::new(),
            pending_handoffs: Vec::new(),
            created_at: "2026-11-15T09:00:00.000Z".into(),
            updated_at: "2026-11-15T09:00:00.000Z".into(),
        }
    }

    fn snoozed(id: &str) -> Thread {
        Thread {
            snoozed_until: Some(LATER.into()),
            snoozed_at: Some(NOW.into()),
            ..thread(id)
        }
    }

    fn model(threads: Vec<Thread>, requests: Vec<ThreadItem>) -> ReadModel {
        ReadModel::from_snapshot(&Snapshot {
            threads,
            pending_requests: requests,
            ..Snapshot::default()
        })
    }

    fn approval(thread_id: &str, created_at: &str) -> ThreadItem {
        ThreadItem {
            id: format!("approval-{thread_id}"),
            thread_id: thread_id.into(),
            created_at: created_at.into(),
            content: ItemContent::Approval {
                tool_call_id: None,
                tool_name: "Bash".into(),
                input: "{}".into(),
                reason: None,
                can_accept_for_session: false,
                status: RequestStatus::Pending,
                decision: None,
                feedback: None,
            },
        }
    }

    fn question(thread_id: &str, created_at: &str) -> ThreadItem {
        ThreadItem {
            id: format!("question-{created_at}"),
            thread_id: thread_id.into(),
            created_at: created_at.into(),
            content: ItemContent::Questions {
                questions: vec![Question {
                    question: "Which one?".into(),
                    header: "Pick".into(),
                    options: Vec::new(),
                    multi_select: false,
                }],
                status: RequestStatus::Pending,
                answers: None,
            },
        }
    }

    fn snooze_command(until: &str) -> Command {
        Command::ThreadSnooze {
            thread_id: "t".into(),
            until: until.into(),
        }
    }

    #[test]
    fn a_snooze_holds_until_its_time_and_needs_no_timer() {
        let thread = snoozed("t");
        let model = model(vec![thread.clone()], Vec::new());

        assert!(is_snoozed(&model, &thread, at("2026-11-15T14:59:59.999Z")));
        assert!(!is_snoozed(&model, &thread, at(LATER)));
        let awake = Thread {
            snoozed_until: None,
            ..thread.clone()
        };
        assert!(!is_snoozed(&model, &awake, at(NOW)));
        assert_eq!(woke_at(&model, &thread, at(NOW)), None);
        assert_eq!(woke_at(&model, &thread, at(LATER)), Some(at(LATER)));
    }

    #[test]
    fn approvals_new_questions_and_new_failures_wake_it_early_but_a_finished_turn_does_not() {
        let now = at("2026-11-15T13:00:00.000Z");
        let finished = Thread {
            last_turn_ended_at: Some("2026-11-15T12:30:00.000Z".into()),
            ..snoozed("t")
        };
        assert!(is_snoozed(
            &model(vec![finished.clone()], Vec::new()),
            &finished,
            now
        ));

        let asking = model(
            vec![snoozed("t")],
            vec![approval("t", "2026-11-15T11:00:00.000Z")],
        );
        assert!(raised_hand_while_snoozed(&asking, &snoozed("t")));
        assert!(!is_snoozed(&asking, &snoozed("t"), now));

        let old_question = model(
            vec![snoozed("t")],
            vec![question("t", "2026-11-15T11:00:00.000Z")],
        );
        assert!(is_snoozed(&old_question, &snoozed("t"), now));
        let new_question = model(
            vec![snoozed("t")],
            vec![question("t", "2026-11-15T12:10:00.000Z")],
        );
        assert!(!is_snoozed(&new_question, &snoozed("t"), now));
        assert_eq!(
            woke_at(&new_question, &snoozed("t"), now),
            Some(at("2026-11-15T12:10:00.000Z"))
        );

        let failed_before = Thread {
            status: ThreadStatus::Error,
            last_turn_ended_at: Some("2026-11-15T11:00:00.000Z".into()),
            ..snoozed("t")
        };
        assert!(is_snoozed(
            &model(vec![failed_before.clone()], Vec::new()),
            &failed_before,
            now
        ));
        let failed_after = Thread {
            last_turn_ended_at: Some("2026-11-15T12:20:00.000Z".into()),
            ..failed_before
        };
        let failing = model(vec![failed_after.clone()], Vec::new());
        assert!(!is_snoozed(&failing, &failed_after, now));
        assert_eq!(
            woke_at(&failing, &failed_after, now),
            Some(at("2026-11-15T12:20:00.000Z"))
        );
    }

    #[test]
    fn a_request_of_a_subagent_or_a_delegated_task_wakes_it_too() {
        let now = at("2026-11-15T13:00:00.000Z");
        let subagent = model(
            vec![snoozed("t")],
            vec![approval("t:toolu_1", "2026-11-15T12:30:00.000Z")],
        );
        assert!(!is_snoozed(&subagent, &snoozed("t"), now));

        let task = Thread {
            parent_thread_id: Some("t".into()),
            ..thread("child")
        };
        let delegated = model(
            vec![snoozed("t"), task],
            vec![question("child", "2026-11-15T11:00:00.000Z")],
        );
        assert!(!is_snoozed(&delegated, &snoozed("t"), now));
    }

    #[test]
    fn the_woke_indicator_shows_until_a_visit_or_a_later_turn_and_dismissing_hides_it() {
        let thread = snoozed("t");
        let mut model = model(vec![thread.clone()], Vec::new());
        let after = at("2026-11-15T16:00:00.000Z");
        assert!(!shows_woke(&model, &thread, at(NOW)));
        assert!(shows_woke(&model, &thread, after));

        let seen_turn = Thread {
            last_turn_ended_at: Some("2026-11-15T15:30:00.000Z".into()),
            ..thread.clone()
        };
        assert!(!shows_woke(&model, &seen_turn, after));
        let visited_before = Thread {
            last_visited_at: Some("2026-11-15T13:00:00.000Z".into()),
            ..thread.clone()
        };
        assert!(shows_woke(&model, &visited_before, after));

        let dismissed = decide(
            &model,
            Command::ThreadWokeDismiss {
                thread_id: "t".into(),
            },
            "2026-11-15T16:00:00.000Z",
        )
        .unwrap();
        assert_eq!(
            dismissed,
            vec![Event::ThreadVisited {
                thread_id: "t".into(),
                at: LATER.into(),
            }]
        );
        for event in &dismissed {
            model.apply(event);
        }
        assert!(!shows_woke(&model, model.thread("t").unwrap(), after));
        assert!(
            decide(
                &model,
                Command::ThreadWokeDismiss {
                    thread_id: "t".into()
                },
                "2026-11-15T16:00:00.000Z",
            )
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn a_visit_is_kept_only_when_it_hides_the_woke_indicator() {
        let visit = Command::ThreadVisit {
            thread_id: "t".into(),
        };
        let awake = model(vec![thread("t")], Vec::new());
        assert!(decide(&awake, visit.clone(), NOW).unwrap().is_empty());

        let mut woken = model(vec![snoozed("t")], Vec::new());
        assert!(decide(&woken, visit.clone(), NOW).unwrap().is_empty());
        let after = "2026-11-15T16:00:00.000Z";
        let events = decide(&woken, visit.clone(), after).unwrap();
        for event in &events {
            woken.apply(event);
        }
        let thread = woken.thread("t").unwrap();
        assert_eq!(thread.last_visited_at.as_deref(), Some(after));
        assert!(!shows_woke(&woken, thread, at(after)));
        assert!(decide(&woken, visit, after).unwrap().is_empty());
    }

    #[test]
    fn snoozing_needs_a_future_time_and_nothing_waiting_on_the_user() {
        let model_with = |requests| model(vec![thread("t")], requests);

        let passed = decide(&model_with(Vec::new()), snooze_command(NOW), NOW);
        assert_eq!(
            passed,
            Err(CommandError::SnoozeTimePassed {
                thread_id: "t".into()
            })
        );
        let garbage = decide(&model_with(Vec::new()), snooze_command("soon"), NOW);
        assert!(matches!(
            garbage,
            Err(CommandError::SnoozeTimePassed { .. })
        ));

        let archived = model(
            vec![Thread {
                archived: true,
                ..thread("t")
            }],
            Vec::new(),
        );
        assert_eq!(
            decide(&archived, snooze_command(LATER), NOW),
            Err(CommandError::ThreadArchived {
                thread_id: "t".into()
            })
        );

        let approval = model_with(vec![approval("t", NOW)]);
        assert_eq!(
            decide(&approval, snooze_command(LATER), NOW),
            Err(CommandError::ThreadWaitingForUser {
                thread_id: "t".into()
            })
        );
        let task = Thread {
            parent_thread_id: Some("t".into()),
            ..thread("child")
        };
        let delegated = model(vec![thread("t"), task], vec![question("child", NOW)]);
        assert!(matches!(
            decide(&delegated, snooze_command(LATER), NOW),
            Err(CommandError::ThreadWaitingForUser { .. })
        ));

        let question = model_with(vec![question("t", NOW)]);
        let events = decide(&question, snooze_command("2026-11-15T16:00:00+01:00"), NOW).unwrap();
        assert_eq!(
            events,
            vec![Event::ThreadSnoozed {
                thread_id: "t".into(),
                until: LATER.into(),
                at: NOW.into(),
            }]
        );
    }

    #[test]
    fn a_running_turn_can_be_snoozed_and_snoozing_again_moves_the_wake() {
        let working = Thread {
            status: ThreadStatus::Working,
            active_turn_id: Some("m1".into()),
            ..snoozed("t")
        };
        let mut model = model(vec![working], Vec::new());
        let events = decide(&model, snooze_command("2026-11-16T09:00:00.000Z"), LATER).unwrap();
        for event in &events {
            model.apply(event);
        }
        let thread = model.thread("t").unwrap();
        assert_eq!(
            thread.snoozed_until.as_deref(),
            Some("2026-11-16T09:00:00.000Z")
        );
        assert_eq!(thread.snoozed_at.as_deref(), Some(LATER));
    }

    fn run(model: &mut ReadModel, command: Command, now: &str) -> Vec<Event> {
        let events = decide(model, command, now).unwrap();
        for event in &events {
            model.apply(event);
        }
        events
    }

    fn woke(events: &[Event]) -> Option<Option<String>> {
        events.iter().find_map(|event| match event {
            Event::ThreadUnsnoozed { woke_at, .. } => Some(woke_at.clone()),
            _ => None,
        })
    }

    #[test]
    fn the_user_s_message_answer_secret_pin_archive_or_wake_end_the_snooze() {
        let message = Command::MessageSend {
            thread_id: "t".into(),
            message_id: "m".into(),
            text: "hi".into(),
            attachments: Vec::new(),
        };
        let mut sent = model(vec![snoozed("t")], Vec::new());
        assert_eq!(woke(&run(&mut sent, message, NOW)), Some(None));
        assert_eq!(sent.thread("t").unwrap().snoozed_until, None);

        let answer = Command::RequestRespond {
            thread_id: "t".into(),
            request_id: "question-2026-11-15T11:00:00.000Z".into(),
            response: RequestResponse::Answers {
                answers: vec!["A".into()],
            },
        };
        let mut answered = model(
            vec![snoozed("t")],
            vec![question("t", "2026-11-15T11:00:00.000Z")],
        );
        assert_eq!(woke(&run(&mut answered, answer, NOW)), Some(None));

        let secret = ThreadItem {
            id: "secret".into(),
            thread_id: "t".into(),
            created_at: NOW.into(),
            content: ItemContent::SecretRequest {
                label: "Token".into(),
                reason: "To push".into(),
                placeholder: None,
                status: RequestStatus::Pending,
                saved: None,
            },
        };
        for saved in [true, false] {
            let mut model = model(vec![snoozed("t")], vec![secret.clone()]);
            let events = run(
                &mut model,
                Command::RequestRespond {
                    thread_id: "t".into(),
                    request_id: "secret".into(),
                    response: RequestResponse::Secret { saved },
                },
                NOW,
            );
            assert_eq!(woke(&events).is_some(), saved);
        }

        let mut pinned = model(vec![snoozed("t")], Vec::new());
        let pin = Command::ThreadPin {
            thread_id: "t".into(),
            pinned: true,
        };
        assert_eq!(woke(&run(&mut pinned, pin, NOW)), Some(Some(NOW.into())));
        assert_eq!(
            pinned.thread("t").unwrap().last_snooze_wake_at.as_deref(),
            Some(NOW)
        );

        let mut archived = model(vec![snoozed("t")], Vec::new());
        let archive = Command::ThreadArchive {
            thread_id: "t".into(),
            archived: true,
        };
        assert_eq!(woke(&run(&mut archived, archive, NOW)), Some(None));

        let mut woken = model(vec![snoozed("t")], Vec::new());
        let wake = Command::ThreadWake {
            thread_id: "t".into(),
        };
        assert_eq!(
            woke(&run(&mut woken, wake.clone(), NOW)),
            Some(Some(NOW.into()))
        );
        assert!(run(&mut woken, wake, NOW).is_empty());
    }

    #[test]
    fn hoshi_s_own_messages_leave_the_snooze_alone() {
        let mut model = model(vec![snoozed("t"), thread("child")], Vec::new());
        let events = crate::send_task_result(&model, "t", "child", "m", "done", NOW).unwrap();
        assert_eq!(woke(&events), None);
        for event in &events {
            model.apply(event);
        }
        assert!(model.thread("t").unwrap().snoozed_until.is_some());
    }

    #[test]
    fn a_usage_limit_snoozes_until_its_reset_unless_it_passed() {
        let model = model(vec![thread("t")], Vec::new());
        assert_eq!(
            snooze_until_reset(&model, "t", LATER, NOW),
            vec![Event::ThreadSnoozed {
                thread_id: "t".into(),
                until: LATER.into(),
                at: NOW.into(),
            }]
        );
        assert!(snooze_until_reset(&model, "t", NOW, LATER).is_empty());
    }

    #[test]
    fn inactivity_counts_from_the_wake_once_it_passed() {
        let thread = Thread {
            updated_at: "2026-11-01T09:00:00.000Z".into(),
            ..snoozed("t")
        };
        assert_eq!(
            active_since(&thread, at(NOW)),
            Some(at("2026-11-01T09:00:00.000Z"))
        );
        assert_eq!(
            active_since(&thread, at("2026-11-16T00:00:00.000Z")),
            Some(at(LATER))
        );
    }

    fn local(text: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(text).unwrap()
    }

    #[test]
    fn the_default_menu_lists_soonest_first_and_hides_a_passed_time_today() {
        // A Sunday, 10:00 at UTC+1.
        let now = local("2026-11-15T10:00:00+01:00");
        let choices = snooze_choices(&default_snooze_presets(), &now);
        let rows: Vec<(&str, SnoozeLabel, bool)> = choices
            .iter()
            .map(|choice| (choice.id.as_str(), choice.label, choice.shows_weekday))
            .collect();
        let hours = |count| {
            SnoozeLabel::In(SnoozeAmount {
                count,
                unit: SnoozeUnit::Hours,
            })
        };
        assert_eq!(
            rows,
            vec![
                ("after:60", hours(1), false),
                ("after:180", hours(3), false),
                ("at:today:18:00", SnoozeLabel::ThisEvening, false),
                ("at:tomorrow:09:00", SnoozeLabel::Tomorrow, false),
            ]
        );
        assert_eq!(choices[2].until, at("2026-11-15T17:00:00.000Z"));
        assert_eq!(choices[3].until, at("2026-11-16T08:00:00.000Z"));

        let evening = local("2026-11-16T19:00:00+01:00");
        let ids: Vec<String> = snooze_choices(&default_snooze_presets(), &evening)
            .into_iter()
            .map(|choice| choice.id)
            .collect();
        assert_eq!(
            ids,
            [
                "after:60",
                "after:180",
                "at:tomorrow:09:00",
                "at:monday:09:00"
            ]
        );
        let monday = snooze_choice(
            &SnoozePreset::At {
                day: SnoozeDay::Monday,
                time: "09:00".into(),
            },
            &evening,
        )
        .unwrap();
        assert_eq!(monday.label, SnoozeLabel::NextWeek);
        assert!(monday.shows_weekday);
        assert_eq!(monday.until, at("2026-11-23T08:00:00.000Z"));
    }

    #[test]
    fn weekday_labels_follow_the_days_ahead() {
        let sunday = local("2026-11-15T10:00:00+00:00");
        let label = |day, now: &DateTime<FixedOffset>| {
            snooze_choice(
                &SnoozePreset::At {
                    day,
                    time: "09:00".into(),
                },
                now,
            )
            .unwrap()
            .label
        };
        assert_eq!(label(SnoozeDay::Monday, &sunday), SnoozeLabel::Tomorrow);
        assert_eq!(
            label(SnoozeDay::Friday, &sunday),
            SnoozeLabel::Weekday(Weekday::Fri)
        );
        assert_eq!(
            label(SnoozeDay::Sunday, &sunday),
            SnoozeLabel::NextWeekday(Weekday::Sun)
        );
        let morning = snooze_choice(
            &SnoozePreset::At {
                day: SnoozeDay::Today,
                time: "11:30".into(),
            },
            &sunday,
        )
        .unwrap();
        assert_eq!(morning.label, SnoozeLabel::ThisMorning);
        let late = snooze_choice(&SnoozePreset::After { minutes: 900 }, &sunday).unwrap();
        assert!(late.shows_weekday);
        assert_eq!(
            amount(90),
            SnoozeAmount {
                count: 90,
                unit: SnoozeUnit::Minutes
            }
        );
        assert_eq!(
            amount(2 * 24 * 60),
            SnoozeAmount {
                count: 2,
                unit: SnoozeUnit::Days
            }
        );
    }

    #[test]
    fn the_menu_keeps_one_row_per_minute_and_at_most_eight_valid_presets() {
        let now = local("2026-11-15T17:00:00+00:00");
        let presets = vec![
            SnoozePreset::After { minutes: 60 },
            SnoozePreset::At {
                day: SnoozeDay::Today,
                time: "18:00".into(),
            },
            SnoozePreset::After { minutes: 0 },
            SnoozePreset::After {
                minutes: MAX_SNOOZE_MINUTES + 1,
            },
            SnoozePreset::At {
                day: SnoozeDay::Today,
                time: "25:00".into(),
            },
        ];
        let choices = snooze_choices(&presets, &now);
        assert_eq!(choices.len(), 1);
        assert_eq!(choices[0].id, "after:60");

        let many: Vec<SnoozePreset> = (1..=10)
            .map(|minutes| SnoozePreset::After { minutes })
            .collect();
        assert_eq!(snooze_choices(&many, &now).len(), MAX_SNOOZE_PRESETS);
        assert!(snooze_choices(&[], &now).is_empty());
    }

    #[test]
    fn a_picked_duration_starts_when_picked_and_a_passed_set_time_is_refused() {
        let shown_at = local("2026-11-15T17:58:00+00:00");
        let presets = default_snooze_presets();
        let shown = snooze_choices(&presets, &shown_at);
        let picked_at = local("2026-11-15T18:05:00+00:00");

        let hour = pick_snooze_choice(&shown, "after:60", &presets, &picked_at).unwrap();
        assert_eq!(hour.until, at("2026-11-15T19:05:00.000Z"));
        assert_eq!(
            pick_snooze_choice(&shown, "at:today:18:00", &presets, &picked_at),
            None
        );
        let tomorrow = pick_snooze_choice(&shown, "at:tomorrow:09:00", &presets, &picked_at);
        assert_eq!(tomorrow.unwrap().until, at("2026-11-16T09:00:00.000Z"));
    }

    #[test]
    fn a_custom_snooze_takes_a_future_local_time_or_a_positive_amount() {
        let now = local("2026-11-15T10:00:00+01:00");
        let typed = |date: &str, time: &str| CustomSnooze::At {
            date: date.into(),
            time: time.into(),
        };
        assert_eq!(
            custom_snooze_until(&typed("2026-11-15", "12:30"), &now),
            Some(at("2026-11-15T11:30:00.000Z"))
        );
        for (date, time) in [
            ("2026-11-15", "09:59"),
            ("2026-02-30", "12:00"),
            ("2026-1-15", "12:00"),
            ("2026-11-15", "9:00"),
        ] {
            assert_eq!(custom_snooze_until(&typed(date, time), &now), None);
        }
        let duration = |amount: &str, unit| CustomSnooze::For {
            amount: amount.into(),
            unit,
        };
        assert_eq!(
            custom_snooze_until(&duration("1.5", SnoozeUnit::Hours), &now),
            Some(at("2026-11-15T10:30:00.000Z"))
        );
        for amount in ["0", "-2", "abc", "inf", "1e300"] {
            assert_eq!(
                custom_snooze_until(&duration(amount, SnoozeUnit::Days), &now),
                None
            );
        }
    }

    #[test]
    fn time_left_rounds_up_to_the_largest_unit() {
        let now = at(NOW);
        assert_eq!(snooze_left(NOW, now), SnoozeLeft::Now);
        assert_eq!(
            snooze_left("2026-11-15T12:00:01.000Z", now),
            SnoozeLeft::Minutes(1)
        );
        assert_eq!(
            snooze_left("2026-11-15T12:45:00.000Z", now),
            SnoozeLeft::Minutes(45)
        );
        assert_eq!(
            snooze_left("2026-11-15T14:10:00.000Z", now),
            SnoozeLeft::Hours(3)
        );
        assert_eq!(
            snooze_left("2026-11-17T11:00:00.000Z", now),
            SnoozeLeft::Days(2)
        );
    }

    #[test]
    fn time_left_that_rounds_up_to_a_whole_unit_reads_in_that_unit() {
        let now = at(NOW);
        assert_eq!(
            snooze_left("2026-11-15T12:59:59.500Z", now),
            SnoozeLeft::Hours(1)
        );
        assert_eq!(
            snooze_left("2026-11-16T11:59:59.500Z", now),
            SnoozeLeft::Days(1)
        );
    }
}
