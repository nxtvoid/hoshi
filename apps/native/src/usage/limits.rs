use crate::m;
use chrono::{DateTime, TimeZone, Utc};
use hoshi_core::{
    BankedGrant, BankedResetFailure, BankedResetSpend, BankedResets, ExtraUsage,
    ProviderAccountStatus, ProviderPlan, ResetKind, SpendUnit, UsageLimits, UsageWindow,
    UsageWindowKind,
};

const MINUTE: i64 = 60_000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

const LOW_PERCENT: u8 = 10;
const WARN_PERCENT: u8 = 20;
/// The shortest window that reads as monthly rather than weekly.
const MONTH_MINUTES: u32 = 28 * 24 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub days: i64,
    pub hours: i64,
    pub minutes: i64,
}

impl Span {
    fn of(millis: i64) -> Self {
        Self {
            days: millis / DAY,
            hours: (millis % DAY) / HOUR,
            minutes: (millis % HOUR) / MINUTE,
        }
    }
}

pub fn remaining_percent(window: &UsageWindow) -> u8 {
    100 - window.used_percent.min(100)
}

/// How close a window is to running out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimitTone {
    Normal,
    Warning,
    Low,
}

pub fn limit_tone(left: u8) -> LimitTone {
    if left <= LOW_PERCENT {
        LimitTone::Low
    } else if left <= WARN_PERCENT {
        LimitTone::Warning
    } else {
        LimitTone::Normal
    }
}

pub fn restored_percent(window: &UsageWindow) -> u8 {
    100 - remaining_percent(window)
}

fn millis_until(time: Option<&str>, now: DateTime<Utc>) -> Option<i64> {
    let at = DateTime::parse_from_rfc3339(time?).ok()?;
    Some((at.timestamp_millis() - now.timestamp_millis()).max(0))
}

/// How much of the window's time is left, 0 to 100: where even spending would leave the quota.
pub fn time_left_percent(window: &UsageWindow, now: DateTime<Utc>) -> Option<f32> {
    let left = millis_until(window.resets_at.as_deref(), now)?;
    let length = i64::from(window.window_minutes) * MINUTE;
    (length > 0).then(|| (left as f64 * 100. / length as f64).min(100.) as f32)
}

pub fn time_to_reset(window: &UsageWindow, now: DateTime<Utc>) -> Option<Span> {
    millis_until(window.resets_at.as_deref(), now).map(Span::of)
}

#[derive(Debug, PartialEq, Eq)]
pub struct ExtraTexts {
    pub shown: String,
    pub note: Option<String>,
    pub detail: String,
}

/// The extra usage row's amount, its note, and the detail beside them: the cap and when it
/// resets, else the cap alone as a monthly one.
pub fn extra_texts(extra: &ExtraUsage, now: DateTime<Utc>) -> ExtraTexts {
    let unit = &extra.unit;
    let texts = |shown, note: Option<String>, detail| ExtraTexts {
        shown,
        note,
        detail,
    };
    match (extra.limit, extra.used, extra.balance) {
        _ if extra.unlimited => texts(m::usage_extra_unlimited(), None, String::new()),
        (Some(limit), Some(used), _) => texts(
            amount(limit.saturating_sub(used), unit),
            Some(m::usage_left()),
            match millis_until(extra.resets_at.as_deref(), now) {
                Some(millis) => {
                    m::usage_extra_cap_resets(amount(limit, unit), format_span(Span::of(millis)))
                }
                None => m::usage_extra_monthly_cap(amount(limit, unit)),
            },
        ),
        (_, _, Some(balance)) => texts(
            amount(balance, unit),
            Some(m::usage_left()),
            m::usage_credits_balance(),
        ),
        (_, used, _) => texts(
            amount(used.unwrap_or(0), unit),
            Some(m::usage_extra_spent()),
            m::usage_extra_no_cap(),
        ),
    }
}

/// Where even spending would leave the quota by now, rounded.
pub fn even_pace_left(window: &UsageWindow, now: DateTime<Utc>) -> Option<u32> {
    time_left_percent(window, now).map(|left| left.round() as u32)
}

/// The banked resets that can be spent now; `None` when there are none to show.
pub fn spendable_resets(limits: Option<&UsageLimits>) -> Option<&BankedResets> {
    limits?
        .banked_resets
        .as_ref()
        .filter(|banked| banked.count > 0 && banked.next_grant_id.is_some())
}

/// Whether the session window covering every model is used up, which a banked reset frees.
pub fn session_spent(limits: Option<&UsageLimits>) -> bool {
    limits.is_some_and(|limits| {
        limits.windows.iter().any(|window| {
            window.kind == UsageWindowKind::Session
                && window.model.is_none()
                && window.used_percent >= 100
        })
    })
}

/// Only the session window covering every model shows the banked resets.
pub fn banked_resets_for<'a>(
    window: &UsageWindow,
    limits: Option<&'a UsageLimits>,
) -> Option<&'a BankedResets> {
    (window.kind == UsageWindowKind::Session && window.model.is_none())
        .then(|| spendable_resets(limits))
        .flatten()
}

/// How long until the soonest banked reset expires.
pub fn time_to_expiry(banked: &BankedResets, now: DateTime<Utc>) -> Option<Span> {
    banked
        .expires_at
        .iter()
        .filter_map(|time| millis_until(Some(time), now))
        .find(|millis| *millis > 0)
        .map(Span::of)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResetDay {
    Today,
    Tomorrow,
    Other,
}

/// When a window resets in `now`'s time zone: the time alone today or tomorrow, the weekday
/// within a week, the date after.
pub fn reset_moment<Tz: TimeZone>(
    window: &UsageWindow,
    now: &DateTime<Tz>,
) -> Option<(ResetDay, String)>
where
    Tz::Offset: std::fmt::Display,
{
    moment_at(window.resets_at.as_deref()?, now)
}

/// An RFC 3339 time in `now`'s time zone, as [`reset_moment`] words it.
pub fn moment_at<Tz: TimeZone>(at: &str, now: &DateTime<Tz>) -> Option<(ResetDay, String)>
where
    Tz::Offset: std::fmt::Display,
{
    let at = DateTime::parse_from_rfc3339(at)
        .ok()?
        .with_timezone(&now.timezone());
    let days = (at.date_naive() - now.date_naive()).num_days();
    let time = "%-I:%M %p";
    Some(match days {
        ..=0 => (ResetDay::Today, at.format(time).to_string()),
        1 => (ResetDay::Tomorrow, at.format(time).to_string()),
        2..7 => (
            ResetDay::Other,
            at.format(&format!("%a {time}")).to_string(),
        ),
        _ => (
            ResetDay::Other,
            at.format(&format!("%b %-d, {time}")).to_string(),
        ),
    })
}

pub fn reset_when((day, text): (ResetDay, String)) -> String {
    match day {
        ResetDay::Today => m::usage_reset_today(text),
        ResetDay::Tomorrow => m::usage_reset_tomorrow(text),
        ResetDay::Other => text,
    }
}

pub fn format_span(span: Span) -> String {
    if span.days > 0 {
        m::duration_days_hours(span.days, span.hours)
    } else if span.hours > 0 {
        m::duration_hours_minutes(span.hours, span.minutes)
    } else {
        m::duration_minutes(span.minutes)
    }
}

pub fn format_expiry(span: Span) -> String {
    if span.days > 0 {
        m::duration_days(span.days as usize)
    } else {
        format_span(span)
    }
}

/// How long until `grant` expires; `None` when it never does or already did.
pub fn grant_expiry(grant: &BankedGrant, now: DateTime<Utc>) -> Option<Span> {
    millis_until(grant.expires_at.as_deref(), now)
        .filter(|millis| *millis > 0)
        .map(Span::of)
}

/// The share of the extra usage cap still unspent; `None` without a cap or what was spent.
pub fn extra_left_percent(extra: &ExtraUsage) -> Option<u8> {
    let (limit, used) = (extra.limit?, extra.used?);
    if limit == 0 {
        return Some(0);
    }
    Some(100 - (used.min(limit).saturating_mul(100) / limit) as u8)
}

/// An amount of `unit`, grouped by thousands, with its fraction only when it has one.
pub fn amount(value: u64, unit: &SpendUnit) -> String {
    let exponent = match unit {
        SpendUnit::Money { exponent, .. } | SpendUnit::Credits { exponent } => *exponent,
    };
    let scale = 10u64.saturating_pow(u32::from(exponent));
    let (whole, fraction) = (value / scale, value % scale);
    let digits = whole.to_string();
    let mut text = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            text.push(',');
        }
        text.push(digit);
    }
    if fraction > 0 {
        text.push_str(&format!(
            ".{fraction:0width$}",
            width = usize::from(exponent)
        ));
    }
    match unit {
        SpendUnit::Credits { .. } => text,
        SpendUnit::Money { currency, .. } => match currency.as_deref().unwrap_or("USD") {
            "USD" => format!("${text}"),
            "EUR" => format!("€{text}"),
            "GBP" => format!("£{text}"),
            "JPY" => format!("¥{text}"),
            code => format!("{code} {text}"),
        },
    }
}

pub fn reset_kind_label(kind: ResetKind) -> String {
    match kind {
        ResetKind::Session => m::usage_reset_kind_session(),
        ResetKind::Weekly => m::usage_reset_kind_weekly(),
        ResetKind::Fable => m::usage_reset_kind_fable(),
        ResetKind::Full => m::usage_reset_kind_full(),
    }
}

/// What spending a reset of `kind` refills.
pub fn reset_kind_description(kind: ResetKind) -> String {
    match kind {
        ResetKind::Session => m::usage_reset_kind_session_description(),
        ResetKind::Weekly => m::usage_reset_kind_weekly_description(),
        ResetKind::Fable => m::usage_reset_kind_fable_description(),
        ResetKind::Full => m::usage_reset_kind_full_description(),
    }
}

pub fn window_label(window: &UsageWindow) -> String {
    match (window.kind, &window.model) {
        (UsageWindowKind::Session, _) => m::usage_session(),
        (UsageWindowKind::Weekly, Some(model)) => m::usage_weekly_model(model),
        (UsageWindowKind::Weekly, None) if window.window_minutes >= MONTH_MINUTES => {
            m::usage_monthly()
        }
        (UsageWindowKind::Weekly, None) => m::usage_weekly(),
    }
}

pub fn plan_label(plan: ProviderPlan) -> String {
    match plan {
        ProviderPlan::Pro => m::provider_plan_pro(),
        ProviderPlan::Max => m::provider_plan_max(),
        ProviderPlan::Max5x => m::provider_plan_max5x(),
        ProviderPlan::Max20x => m::provider_plan_max20x(),
        ProviderPlan::Team => m::provider_plan_team(),
        ProviderPlan::Enterprise => m::provider_plan_enterprise(),
    }
}

pub fn spend_message(result: BankedResetSpend) -> String {
    match result {
        BankedResetSpend::Reset => m::usage_reset_done(),
        BankedResetSpend::NothingToReset => m::usage_reset_nothing_to_reset(),
        BankedResetSpend::AlreadyUsed => m::usage_reset_already_used(),
        BankedResetSpend::NoneLeft => m::usage_reset_none_left(),
        BankedResetSpend::Failed { reason } => match reason {
            BankedResetFailure::LoginUnreadable => m::usage_reset_login_unreadable(),
            BankedResetFailure::SignedOut => m::usage_reset_signed_out(),
            BankedResetFailure::RateLimited => m::usage_reset_rate_limited(),
            BankedResetFailure::CoolingDown => m::usage_reset_cooling_down(),
            BankedResetFailure::Unconfirmed => m::usage_reset_unconfirmed(),
            BankedResetFailure::RequestFailed => m::usage_reset_request_failed(),
            BankedResetFailure::LimitsUnread => m::usage_reset_limits_unread(),
        },
    }
}

/// What a provider's section shows under its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionState {
    Limits,
    /// The plan has no windows, as with API keys.
    NoLimits,
    Checking,
    /// It was asked and couldn't say.
    Unavailable,
    SignedOut,
    NotInstalled,
}

/// A provider that can't run says so even over the limits it reported before.
pub fn section_state(
    account: Option<ProviderAccountStatus>,
    limits: Option<&UsageLimits>,
    checking: bool,
    failed: bool,
) -> SectionState {
    match (account, limits) {
        (Some(ProviderAccountStatus::NotInstalled), _) => SectionState::NotInstalled,
        (Some(ProviderAccountStatus::SignedOut), _) => SectionState::SignedOut,
        (_, Some(limits)) if limits.windows.is_empty() && limits.extra_usage.is_none() => {
            SectionState::NoLimits
        }
        (_, Some(_)) => SectionState::Limits,
        (_, None) if failed && !checking => SectionState::Unavailable,
        (_, None) => SectionState::Checking,
    }
}

/// The email with its letters hidden, so it stays private while sharing the screen.
pub fn masked_email(email: &str) -> String {
    email
        .chars()
        .map(|c| if c == '@' || c == '.' { c } else { '•' })
        .collect()
}

#[cfg(test)]
mod tests;
