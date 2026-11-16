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

pub const LOW_PERCENT: u8 = 10;
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
        (_, Some(limits)) if limits.windows.is_empty() => SectionState::NoLimits,
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
mod tests {
    use super::{
        ResetDay, SectionState, Span, amount, banked_resets_for, even_pace_left,
        extra_left_percent, format_expiry, format_span, masked_email, remaining_percent,
        reset_moment, restored_percent, section_state, session_spent, spendable_resets,
        time_left_percent, time_to_expiry, time_to_reset, window_label,
    };
    use chrono::{DateTime, FixedOffset, TimeZone, Utc};
    use hoshi_core::{
        BankedResets, ExtraUsage, ProviderAccountStatus, SpendUnit, UsageLimits, UsageWindow,
        UsageWindowKind,
    };

    #[test]
    fn amounts_show_their_fraction_only_when_they_have_one() {
        let money = |currency: Option<&str>, exponent| SpendUnit::Money {
            currency: currency.map(str::to_owned),
            exponent,
        };
        assert_eq!(amount(3_800, &money(Some("USD"), 2)), "$38");
        assert_eq!(amount(1_234_567, &money(None, 2)), "$12,345.67");
        assert_eq!(amount(5, &money(Some("EUR"), 2)), "€0.05");
        assert_eq!(amount(1_500, &money(Some("JPY"), 0)), "¥1,500");
        assert_eq!(amount(10_000, &money(Some("CAD"), 2)), "CAD 100");
        assert_eq!(
            amount(51_250, &SpendUnit::Credits { exponent: 2 }),
            "512.50"
        );
    }

    #[test]
    fn extra_usage_is_left_as_a_share_of_its_cap() {
        let extra = |used, limit| ExtraUsage {
            unit: SpendUnit::Credits { exponent: 2 },
            used,
            limit,
            balance: None,
            unlimited: false,
            resets_at: None,
        };
        assert_eq!(
            extra_left_percent(&extra(Some(3_800), Some(10_000))),
            Some(62)
        );
        assert_eq!(
            extra_left_percent(&extra(Some(12_000), Some(10_000))),
            Some(0)
        );
        assert_eq!(extra_left_percent(&extra(Some(0), Some(0))), Some(0));
        assert_eq!(extra_left_percent(&extra(Some(500), None)), None);
        assert_eq!(extra_left_percent(&extra(None, Some(500))), None);
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 13, 12, 0, 0).unwrap()
    }

    fn window(kind: UsageWindowKind, model: Option<&str>, resets_at: Option<&str>) -> UsageWindow {
        UsageWindow {
            kind,
            model: model.map(str::to_owned),
            used_percent: 30,
            window_minutes: 300,
            resets_at: resets_at.map(str::to_owned),
        }
    }

    fn banked(count: u32, grant: Option<&str>) -> BankedResets {
        BankedResets {
            count,
            next_grant_id: grant.map(str::to_owned),
            expires_at: vec!["2026-11-13T11:00:00Z".into(), "2026-11-15T14:30:00Z".into()],
            grants: Vec::new(),
        }
    }

    #[test]
    fn percents_are_clamped_and_complement_each_other() {
        let mut session = window(UsageWindowKind::Session, None, None);
        assert_eq!(remaining_percent(&session), 70);
        assert_eq!(restored_percent(&session), 30);
        session.used_percent = 140;
        assert_eq!(remaining_percent(&session), 0);
        assert_eq!(restored_percent(&session), 100);
    }

    #[test]
    fn the_pace_mark_follows_the_time_left_in_the_window() {
        let session = window(UsageWindowKind::Session, None, Some("2026-11-13T13:30:00Z"));
        assert_eq!(time_left_percent(&session, now()), Some(30.));
        assert_eq!(even_pace_left(&session, now()), Some(30));
        assert_eq!(
            time_to_reset(&session, now()),
            Some(Span {
                days: 0,
                hours: 1,
                minutes: 30
            })
        );
        let unknown = window(UsageWindowKind::Session, None, None);
        assert_eq!(time_left_percent(&unknown, now()), None);
        let past = window(UsageWindowKind::Session, None, Some("2026-11-13T10:00:00Z"));
        assert_eq!(time_left_percent(&past, now()), Some(0.));
    }

    #[test]
    fn only_a_used_up_session_window_for_every_model_is_spent() {
        let spent = |kind, model: Option<&str>, used_percent| {
            let limits = UsageLimits {
                windows: vec![UsageWindow {
                    used_percent,
                    ..window(kind, model, None)
                }],
                banked_resets: None,
                account: None,
                extra_usage: None,
            };
            session_spent(Some(&limits))
        };
        assert!(spent(UsageWindowKind::Session, None, 100));
        assert!(!spent(UsageWindowKind::Session, None, 99));
        assert!(!spent(UsageWindowKind::Weekly, None, 100));
        assert!(!spent(UsageWindowKind::Session, Some("Fable"), 100));
        assert!(!session_spent(None));
    }

    #[test]
    fn only_the_session_window_for_every_model_shows_spendable_resets() {
        let limits = UsageLimits {
            windows: Vec::new(),
            banked_resets: Some(banked(2, Some("grant"))),
            account: None,
            extra_usage: None,
        };
        let session = window(UsageWindowKind::Session, None, None);
        assert!(banked_resets_for(&session, Some(&limits)).is_some());
        let weekly = window(UsageWindowKind::Weekly, None, None);
        assert!(banked_resets_for(&weekly, Some(&limits)).is_none());
        let model = window(UsageWindowKind::Session, Some("Fable"), None);
        assert!(banked_resets_for(&model, Some(&limits)).is_none());

        let no_grant = UsageLimits {
            banked_resets: Some(banked(2, None)),
            ..limits.clone()
        };
        assert!(spendable_resets(Some(&no_grant)).is_none());
        let none_left = UsageLimits {
            banked_resets: Some(banked(0, Some("grant"))),
            ..limits
        };
        assert!(spendable_resets(Some(&none_left)).is_none());
        assert!(spendable_resets(None).is_none());
    }

    #[test]
    fn the_expiry_is_the_soonest_one_still_ahead() {
        assert_eq!(
            time_to_expiry(&banked(2, Some("grant")), now()),
            Some(Span {
                days: 2,
                hours: 2,
                minutes: 30
            })
        );
    }

    #[test]
    fn a_reset_reads_as_today_tomorrow_a_weekday_or_a_date() {
        let zone = FixedOffset::west_opt(3 * 3600).unwrap();
        let now = now().with_timezone(&zone);
        let at = |time: &str| window(UsageWindowKind::Session, None, Some(time));
        assert_eq!(
            reset_moment(&at("2026-11-13T18:05:00Z"), &now),
            Some((ResetDay::Today, "3:05 PM".into()))
        );
        assert_eq!(
            reset_moment(&at("2026-11-14T02:30:00Z"), &now),
            Some((ResetDay::Today, "11:30 PM".into()))
        );
        assert_eq!(
            reset_moment(&at("2026-11-14T12:00:00Z"), &now),
            Some((ResetDay::Tomorrow, "9:00 AM".into()))
        );
        assert_eq!(
            reset_moment(&at("2026-11-17T12:00:00Z"), &now),
            Some((ResetDay::Other, "Tue 9:00 AM".into()))
        );
        assert_eq!(
            reset_moment(&at("2026-11-24T12:00:00Z"), &now),
            Some((ResetDay::Other, "Nov 24, 9:00 AM".into()))
        );
        assert_eq!(reset_moment(&at("soon"), &now), None);
    }

    #[test]
    fn durations_show_their_two_largest_units() {
        let span = |days, hours, minutes| Span {
            days,
            hours,
            minutes,
        };
        assert_eq!(format_span(span(2, 3, 15)), "2d 3h");
        assert_eq!(format_span(span(0, 3, 15)), "3h 15m");
        assert_eq!(format_span(span(0, 0, 7)), "7m");
        assert_eq!(format_expiry(span(1, 3, 15)), "1 day");
        assert_eq!(format_expiry(span(3, 3, 15)), "3 days");
        assert_eq!(format_expiry(span(0, 3, 15)), "3h 15m");
    }

    #[test]
    fn windows_are_named_by_kind_and_model() {
        assert_eq!(
            window_label(&window(UsageWindowKind::Session, None, None)),
            "Session"
        );
        assert_eq!(
            window_label(&window(UsageWindowKind::Weekly, None, None)),
            "Weekly"
        );
        assert_eq!(
            window_label(&window(UsageWindowKind::Weekly, Some("Fable"), None)),
            "Weekly · Fable"
        );
        let month = UsageWindow {
            window_minutes: 30 * 24 * 60,
            ..window(UsageWindowKind::Weekly, None, None)
        };
        assert_eq!(window_label(&month), "Monthly");
    }

    #[test]
    fn a_section_shows_rows_a_note_or_waits() {
        let used_up = UsageLimits {
            windows: vec![UsageWindow {
                used_percent: 100,
                ..window(UsageWindowKind::Session, None, None)
            }],
            ..UsageLimits::default()
        };
        let ready = Some(ProviderAccountStatus::Ready);
        assert_eq!(
            section_state(ready, Some(&used_up), false, false),
            SectionState::Limits
        );
        assert_eq!(remaining_percent(&used_up.windows[0]), 0);
        assert_eq!(
            section_state(None, Some(&UsageLimits::default()), false, true),
            SectionState::NoLimits
        );
        assert_eq!(
            section_state(
                Some(ProviderAccountStatus::SignedOut),
                Some(&used_up),
                false,
                false
            ),
            SectionState::SignedOut
        );
        assert_eq!(
            section_state(Some(ProviderAccountStatus::NotInstalled), None, true, false),
            SectionState::NotInstalled
        );
        assert_eq!(
            section_state(ready, None, false, true),
            SectionState::Unavailable
        );
        assert_eq!(
            section_state(ready, None, true, true),
            SectionState::Checking
        );
        assert_eq!(
            section_state(None, None, false, false),
            SectionState::Checking
        );
    }

    #[test]
    fn a_masked_email_keeps_only_its_shape() {
        assert_eq!(masked_email("nx@ex.io"), "••@••.••");
    }
}
