use super::{
    ExtraTexts, LimitTone, ResetDay, SectionState, Span, amount, banked_resets_for, even_pace_left,
    extra_left_percent, extra_texts, format_expiry, format_span, limit_tone, masked_email,
    remaining_percent, reset_moment, restored_percent, section_state, session_spent,
    spendable_resets, time_left_percent, time_to_expiry, time_to_reset, window_label,
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

#[test]
fn a_capped_extra_usage_tells_what_is_left_beside_its_cap() {
    let mut extra = ExtraUsage {
        unit: SpendUnit::Money {
            currency: Some("USD".into()),
            exponent: 2,
        },
        used: Some(4_888),
        limit: Some(10_000),
        balance: None,
        unlimited: false,
        resets_at: None,
    };
    assert_eq!(
        extra_texts(&extra, now()),
        ExtraTexts {
            shown: "$51.12".into(),
            note: Some("left".into()),
            detail: "$100 / month".into(),
        }
    );
    extra.resets_at = Some("2026-11-25T16:00:00Z".into());
    assert_eq!(extra_texts(&extra, now()).detail, "$100 · 12d 4h");
}

#[test]
fn a_window_warns_at_a_fifth_left_and_runs_low_at_a_tenth() {
    assert_eq!(limit_tone(100), LimitTone::Normal);
    assert_eq!(limit_tone(21), LimitTone::Normal);
    assert_eq!(limit_tone(20), LimitTone::Warning);
    assert_eq!(limit_tone(11), LimitTone::Warning);
    assert_eq!(limit_tone(10), LimitTone::Low);
    assert_eq!(limit_tone(0), LimitTone::Low);
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
    let credits_only = UsageLimits {
        extra_usage: Some(ExtraUsage {
            unit: SpendUnit::Credits { exponent: 2 },
            used: None,
            limit: None,
            balance: Some(500),
            unlimited: false,
            resets_at: None,
        }),
        ..UsageLimits::default()
    };
    assert_eq!(
        section_state(ready, Some(&credits_only), false, false),
        SectionState::Limits,
        "a credit balance alone is still shown"
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
