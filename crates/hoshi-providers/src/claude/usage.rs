//! Claude's plan limits: asked of the CLI on demand, and reported by it mid-turn.

use super::control;
use chrono::{DateTime, SecondsFormat, Utc};
use hoshi_core::{ExtraUsage, SpendUnit, UsageLimits, UsageWindow, UsageWindowKind};
use serde_json::{Value, json};

const SESSION_MINUTES: u32 = 5 * 60;
const WEEK_MINUTES: u32 = 7 * 24 * 60;
const REQUEST_ID: &str = "hoshi-usage";

/// Asks a CLI started only for this, from outside any project.
pub(crate) fn probe(binary: &str) -> Option<UsageLimits> {
    // Without it the CLI also scans every local transcript, which grows with history.
    let request = json!({ "subtype": "get_usage", "skip_behaviors": true });
    let usage = control::ask(binary, &std::env::temp_dir(), REQUEST_ID, request)?;
    Some(usage_limits(&usage))
}

/// The CLI's answer to `get_usage`.
fn usage_limits(usage: &Value) -> UsageLimits {
    if usage["rate_limits_available"] != true {
        return UsageLimits::default();
    }
    let limits = &usage["rate_limits"];
    let mut windows: Vec<_> = [
        ("five_hour", UsageWindowKind::Session, SESSION_MINUTES),
        ("seven_day", UsageWindowKind::Weekly, WEEK_MINUTES),
    ]
    .into_iter()
    .filter_map(|(key, kind, window_minutes)| {
        let window = &limits[key];
        Some(UsageWindow {
            kind,
            model: None,
            used_percent: percent(window["utilization"].as_f64()?),
            window_minutes,
            resets_at: window["resets_at"].as_str().and_then(normalize_time),
        })
    })
    .collect();
    let scoped = limits["model_scoped"].as_array().into_iter().flatten();
    windows.extend(scoped.filter_map(|window| {
        Some(UsageWindow {
            kind: UsageWindowKind::Weekly,
            model: Some(window["display_name"].as_str()?.to_owned()),
            used_percent: percent(window["utilization"].as_f64()?),
            window_minutes: WEEK_MINUTES,
            resets_at: window["resets_at"].as_str().and_then(normalize_time),
        })
    }));
    UsageLimits {
        windows,
        banked_resets: None,
        account: None,
        extra_usage: extra_usage(limits),
    }
}

/// What the plan spends past its limits: its spend budget when it has one, which tells the
/// exact amounts, else its extra usage, in cents. `None` unless one is turned on.
fn extra_usage(limits: &Value) -> Option<ExtraUsage> {
    spend_budget(&limits["spend"]).or_else(|| extra_credits(&limits["extra_usage"]))
}

fn spend_budget(spend: &Value) -> Option<ExtraUsage> {
    if spend["enabled"] != true {
        return None;
    }
    let money = |amount: &Value| {
        Some((
            amount["amount_minor"].as_u64()?,
            currency(&amount["currency"])?,
            u8::try_from(amount["exponent"].as_u64()?).ok()?,
        ))
    };
    let (used, currency, exponent) = money(&spend["used"])?;
    let (limit, limit_currency, limit_exponent) = money(&spend["limit"])?;
    if (&limit_currency, limit_exponent) != (&currency, exponent) || exponent > MAX_EXPONENT {
        return None;
    }
    Some(ExtraUsage {
        unit: SpendUnit::Money {
            currency: Some(currency),
            exponent,
        },
        used: Some(used),
        limit: Some(limit),
        balance: None,
        unlimited: false,
        resets_at: None,
    })
}

fn extra_credits(extra: &Value) -> Option<ExtraUsage> {
    if extra["is_enabled"] != true {
        return None;
    }
    let amount = |value: &Value| {
        value
            .as_f64()
            .filter(|amount| amount.is_finite() && *amount >= 0.0)
            .map(|amount| amount.round() as u64)
    };
    Some(ExtraUsage {
        unit: SpendUnit::Money {
            currency: currency(&extra["currency"]),
            exponent: 2,
        },
        used: Some(amount(&extra["used_credits"]).unwrap_or(0)),
        limit: amount(&extra["monthly_limit"]),
        balance: None,
        unlimited: false,
        resets_at: None,
    })
}

const MAX_EXPONENT: u8 = 6;

/// An ISO 4217 code, upper case.
fn currency(code: &Value) -> Option<String> {
    code.as_str()
        .map(str::trim)
        .filter(|code| code.len() == 3 && code.bytes().all(|byte| byte.is_ascii_alphabetic()))
        .map(str::to_ascii_uppercase)
}

/// A `rate_limit_event`'s windows. Its utilization runs from 0 to 1.
pub(crate) fn rate_limit_event(info: &Value) -> Option<UsageLimits> {
    let unified = &info["unifiedWindows"];
    let windows: Vec<_> = [
        ("five_hour", UsageWindowKind::Session, SESSION_MINUTES),
        ("seven_day", UsageWindowKind::Weekly, WEEK_MINUTES),
    ]
    .into_iter()
    .filter_map(|(key, kind, window_minutes)| {
        let window = &unified[key];
        Some(UsageWindow {
            kind,
            model: None,
            used_percent: percent(window["utilization"].as_f64()? * 100.0),
            window_minutes,
            resets_at: window["resetsAt"].as_i64().and_then(epoch_time),
        })
    })
    .collect();
    (!windows.is_empty()).then_some(UsageLimits {
        windows,
        banked_resets: None,
        account: None,
        extra_usage: None,
    })
}

fn percent(value: f64) -> u8 {
    value.round().clamp(0.0, 100.0) as u8
}

fn normalize_time(text: &str) -> Option<String> {
    let time = DateTime::parse_from_rfc3339(text).ok()?;
    Some(
        time.with_timezone(&Utc)
            .to_rfc3339_opts(SecondsFormat::Secs, true),
    )
}

pub(super) fn epoch_time(seconds: i64) -> Option<String> {
    if seconds <= 0 {
        return None;
    }
    let time = DateTime::<Utc>::from_timestamp(seconds, 0)?;
    Some(time.to_rfc3339_opts(SecondsFormat::Secs, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage_response(line: &str) -> Option<UsageLimits> {
        control::response(line, REQUEST_ID).map(|usage| usage_limits(&usage))
    }

    const GET_USAGE: &str = r#"{"type":"control_response","response":{"subtype":"success","request_id":"hoshi-usage","response":{"subscription_type":"max","rate_limits_available":true,"rate_limits":{"five_hour":{"utilization":35,"resets_at":"2026-11-08T00:50:00.093246+00:00"},"seven_day":{"utilization":16.4,"resets_at":"2026-11-12T14:00:00.093266+00:00"},"seven_day_opus":null,"model_scoped":[{"display_name":"Fable","utilization":0,"resets_at":"2026-11-12T14:00:00+00:00"}]}}}}"#;

    #[test]
    fn get_usage_lists_session_weekly_and_model_windows() {
        let limits = usage_response(GET_USAGE).unwrap();

        assert_eq!(
            limits.windows,
            [
                UsageWindow {
                    kind: UsageWindowKind::Session,
                    model: None,
                    used_percent: 35,
                    window_minutes: 300,
                    resets_at: Some("2026-11-08T00:50:00Z".into()),
                },
                UsageWindow {
                    kind: UsageWindowKind::Weekly,
                    model: None,
                    used_percent: 16,
                    window_minutes: 10_080,
                    resets_at: Some("2026-11-12T14:00:00Z".into()),
                },
                UsageWindow {
                    kind: UsageWindowKind::Weekly,
                    model: Some("Fable".into()),
                    used_percent: 0,
                    window_minutes: 10_080,
                    resets_at: Some("2026-11-12T14:00:00Z".into()),
                },
            ]
        );
    }

    fn with_limits(extra: &str) -> Option<ExtraUsage> {
        let line = GET_USAGE.replace(
            r#""seven_day_opus":null"#,
            &format!(r#""seven_day_opus":null,{extra}"#),
        );
        usage_response(&line).unwrap().extra_usage
    }

    fn dollars(used: u64, limit: Option<u64>, currency: Option<&str>) -> ExtraUsage {
        ExtraUsage {
            unit: SpendUnit::Money {
                currency: currency.map(str::to_owned),
                exponent: 2,
            },
            used: Some(used),
            limit,
            balance: None,
            unlimited: false,
            resets_at: None,
        }
    }

    #[test]
    fn get_usage_reads_extra_usage_only_when_it_is_on() {
        assert_eq!(
            with_limits(
                r#""extra_usage":{"is_enabled":true,"monthly_limit":10000,"used_credits":3812.4,"utilization":38.1,"currency":"usd"}"#
            ),
            Some(dollars(3812, Some(10_000), Some("USD")))
        );
        assert_eq!(
            with_limits(
                r#""extra_usage":{"is_enabled":true,"monthly_limit":null,"used_credits":500,"utilization":null}"#
            ),
            Some(dollars(500, None, None))
        );
        assert_eq!(
            with_limits(
                r#""extra_usage":{"is_enabled":false,"monthly_limit":10000,"used_credits":0,"utilization":0}"#
            ),
            None
        );
        assert_eq!(usage_response(GET_USAGE).unwrap().extra_usage, None);
    }

    #[test]
    fn a_spend_budget_tells_the_exact_amounts_over_extra_usage() {
        let money = |amount: u64, currency: &str| {
            format!(r#"{{"amount_minor":{amount},"currency":"{currency}","exponent":2}}"#)
        };
        let budget = |used: &str, limit: &str| {
            format!(
                r#""spend":{{"enabled":true,"used":{used},"limit":{limit}}},"extra_usage":{{"is_enabled":true,"monthly_limit":1,"used_credits":1}}"#
            )
        };

        assert_eq!(
            with_limits(&budget(&money(2_550, "EUR"), &money(20_000, "EUR"))),
            Some(dollars(2_550, Some(20_000), Some("EUR")))
        );
        assert_eq!(
            with_limits(&budget(&money(2_550, "EUR"), &money(20_000, "USD"))),
            Some(dollars(1, Some(1), None))
        );
    }

    #[test]
    fn a_plan_without_limits_has_no_windows() {
        let line = r#"{"type":"control_response","response":{"subtype":"success","request_id":"hoshi-usage","response":{"rate_limits_available":false,"rate_limits":null}}}"#;

        assert_eq!(usage_response(line), Some(UsageLimits::default()));
    }

    #[test]
    fn other_lines_and_failed_requests_are_not_an_answer() {
        let failed = r#"{"type":"control_response","response":{"subtype":"error","request_id":"hoshi-usage","error":"nope"}}"#;

        assert_eq!(usage_response(failed), None);
        assert_eq!(usage_response(r#"{"type":"system"}"#), None);
        assert_eq!(usage_response("not json"), None);
    }

    #[test]
    fn a_rate_limit_event_reports_its_unified_windows() {
        let info = json!({
            "status": "allowed",
            "unifiedWindows": {
                "five_hour": { "utilization": 0.344, "resetsAt": 1_794_099_000 },
                "seven_day": { "utilization": 0.16, "resetsAt": 1_794_492_000 },
            },
        });

        let limits = rate_limit_event(&info).unwrap();

        assert_eq!(limits.windows[0].kind, UsageWindowKind::Session);
        assert_eq!(limits.windows[0].used_percent, 34);
        assert_eq!(
            limits.windows[0].resets_at.as_deref(),
            Some("2026-11-08T00:50:00Z")
        );
        assert_eq!(limits.windows[1].used_percent, 16);
        assert_eq!(rate_limit_event(&json!({ "status": "allowed" })), None);
    }
}
