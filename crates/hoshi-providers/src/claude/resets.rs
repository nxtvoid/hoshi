//! Session resets a Claude plan banked: read with the limits, and spent on request.

use super::login::{access_token, config_dir, organization_id};
use chrono::{DateTime, SecondsFormat, Utc};
use hoshi_core::{BankedGrant, BankedResetFailure, BankedResetSpend, BankedResets, ResetKind};
use serde_json::{Value, json};
use std::time::Duration;

const API_BASE: &str = "https://api.anthropic.com";
const PROGRAM: &str = "cedar_ember";
const READ_TIMEOUT: Duration = Duration::from_secs(10);
const SPEND_TIMEOUT: Duration = Duration::from_secs(25);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Method {
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Request {
    method: Method,
    url: String,
    headers: Vec<(&'static str, String)>,
    body: Option<String>,
}

struct Response {
    status: u16,
    /// `Null` when the body isn't JSON.
    body: Value,
}

/// `None` when the login lives in the keychain, the plan can't bank resets, or the
/// request fails.
pub(crate) fn read(version: &str) -> Option<BankedResets> {
    let token = access_token(&config_dir()?).ok()??;
    let response = send(&read_request(&token, version), READ_TIMEOUT)?;
    if !(200..300).contains(&response.status) {
        return None;
    }
    banked_resets(&response.body["cedar_ember"], Utc::now())
}

/// `request_id` makes retrying the same spend safe: the provider spends once per id.
pub(crate) fn spend(version: &str, grant_id: &str, request_id: &str) -> BankedResetSpend {
    if !valid_grant_id(grant_id) || !valid_request_id(request_id) {
        return BankedResetSpend::NoneLeft;
    }
    let failed = |reason| BankedResetSpend::Failed { reason };
    let Some(dir) = config_dir() else {
        return failed(BankedResetFailure::SignedOut);
    };
    let (Ok(token), Ok(organization)) = (access_token(&dir), organization_id()) else {
        return failed(BankedResetFailure::LoginUnreadable);
    };
    let (Some(token), Some(organization)) = (token, organization) else {
        return failed(BankedResetFailure::SignedOut);
    };
    let request = spend_request(&token, &organization, version, grant_id, request_id);
    match send(&request, SPEND_TIMEOUT) {
        Some(response) => spend_outcome(response.status, &response.body),
        None => failed(BankedResetFailure::RequestFailed),
    }
}

fn headers(token: &str, version: &str) -> Vec<(&'static str, String)> {
    vec![
        ("authorization", format!("Bearer {token}")),
        ("anthropic-beta", "oauth-2025-04-20".into()),
        (
            "user-agent",
            format!("claude-cli/{version} (external, cli)"),
        ),
    ]
}

fn read_request(token: &str, version: &str) -> Request {
    Request {
        method: Method::Get,
        url: format!("{API_BASE}/api/oauth/usage?cedar_ember=1&skip_spend=1"),
        headers: headers(token, version),
        body: None,
    }
}

fn spend_request(
    token: &str,
    organization: &str,
    version: &str,
    grant_id: &str,
    request_id: &str,
) -> Request {
    let mut headers = headers(token, version);
    headers.push(("content-type", "application/json".into()));
    let body = json!({ "program": PROGRAM, "grant_id": grant_id, "request_id": request_id });
    Request {
        method: Method::Post,
        url: format!(
            "{API_BASE}/api/organizations/{}/reset_rate_limits",
            encode_segment(organization)
        ),
        headers,
        body: Some(body.to_string()),
    }
}

fn spend_outcome(status: u16, body: &Value) -> BankedResetSpend {
    let failed = |reason| BankedResetSpend::Failed { reason };
    match status {
        429 => return failed(BankedResetFailure::RateLimited),
        401 | 403 => return failed(BankedResetFailure::SignedOut),
        200..=299 => {}
        _ => return failed(BankedResetFailure::RequestFailed),
    }
    match body["result"].as_str() {
        Some("reset") => BankedResetSpend::Reset,
        Some("not_limited") => BankedResetSpend::NothingToReset,
        Some("already_used") => BankedResetSpend::AlreadyUsed,
        Some("ineligible") => BankedResetSpend::NoneLeft,
        Some("cooldown") => failed(BankedResetFailure::CoolingDown),
        Some("unavailable") => failed(BankedResetFailure::Unconfirmed),
        _ => failed(BankedResetFailure::RequestFailed),
    }
}

/// The usage answer's `cedar_ember` block. Counts nothing unless the grant to spend next
/// is among the usable ones.
fn banked_resets(block: &Value, now: DateTime<Utc>) -> Option<BankedResets> {
    if !block["eligible"].as_bool()? {
        return None;
    }
    let grants = match &block["grants"] {
        Value::Null => &Vec::new(),
        Value::Array(grants) => grants,
        _ => return None,
    };
    let usable: Vec<_> = grants
        .iter()
        .filter_map(|grant| usable_grant(grant, now))
        .collect();
    let next = block["next_grant_id"]
        .as_str()
        .filter(|id| usable.iter().any(|grant| grant.id == *id));
    let Some(next) = next else {
        return Some(BankedResets::default());
    };
    let mut expires_at: Vec<_> = usable
        .iter()
        .filter(|grant| grant.resets > 0)
        .filter_map(|grant| grant.expires_at.clone())
        .collect();
    expires_at.sort();
    Some(BankedResets {
        count: usable.iter().map(|grant| grant.resets).sum(),
        next_grant_id: Some(next.to_owned()),
        expires_at,
        grants: by_kind(&usable, next),
    })
}

/// One entry per kind of reset left, as Claude Code groups them, spending the next grant when
/// it is of that kind, else the one that expires first.
fn by_kind(usable: &[Grant<'_>], next: &str) -> Vec<BankedGrant> {
    [
        ResetKind::Session,
        ResetKind::Weekly,
        ResetKind::Fable,
        ResetKind::Full,
    ]
    .into_iter()
    .filter_map(|kind| {
        let mut grants: Vec<_> = usable
            .iter()
            .filter(|grant| grant.kind == kind && grant.resets > 0)
            .collect();
        grants.sort_by_key(|grant| {
            (
                grant.id != next,
                grant.expires_at.is_none(),
                grant.expires_at.clone(),
            )
        });
        let target = grants.first()?;
        Some(BankedGrant {
            id: target.id.to_owned(),
            kind,
            resets: grants.iter().map(|grant| grant.resets).sum(),
            expires_at: target.expires_at.clone(),
        })
    })
    .collect()
}

struct Grant<'a> {
    id: &'a str,
    kind: ResetKind,
    resets: u32,
    expires_at: Option<String>,
}

/// What a grant refills: a Fable reset when it gives back weekly usage, else its declared
/// type, else what it clears.
fn reset_kind(grant: &Value) -> ResetKind {
    if grant["gives_back_weekly"] == true {
        return ResetKind::Fable;
    }
    match grant["type"].as_str() {
        Some("session") => return ResetKind::Session,
        Some("weekly") => return ResetKind::Weekly,
        Some("full") => return ResetKind::Full,
        _ => {}
    }
    let clears: Vec<&str> = grant["clears"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let session = clears.contains(&"five_hour");
    let other = clears.iter().any(|limit| *limit != "five_hour");
    match (session, other) {
        (true, false) => ResetKind::Session,
        (false, true) => ResetKind::Weekly,
        _ => ResetKind::Full,
    }
}

fn usable_grant(grant: &Value, now: DateTime<Utc>) -> Option<Grant<'_>> {
    let id = grant["id"].as_str().filter(|id| valid_grant_id(id))?;
    let resets = u32::try_from(grant["resets_left"].as_u64()?).ok()?;
    if grant["paused"] == true || grant["usable_now"] != true {
        return None;
    }
    let expires_at = match &grant["ends_at"] {
        Value::Null => None,
        Value::String(text) => Some(future_time(text, now)?),
        _ => return None,
    };
    Some(Grant {
        id,
        kind: reset_kind(grant),
        resets,
        expires_at,
    })
}

/// A complete RFC 3339 time after `now`, in UTC.
fn future_time(text: &str, now: DateTime<Utc>) -> Option<String> {
    if text.as_bytes().get(10) != Some(&b'T') {
        return None;
    }
    let time = DateTime::parse_from_rfc3339(text).ok()?.with_timezone(&Utc);
    (time > now).then(|| time.to_rfc3339_opts(SecondsFormat::Secs, true))
}

fn valid_grant_id(id: &str) -> bool {
    (1..=40).contains(&id.len())
        && id
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
}

fn valid_request_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn encode_segment(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn send(request: &Request, timeout: Duration) -> Option<Response> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .http_status_as_error(false)
        .build()
        .into();
    let result = match request.method {
        Method::Get => {
            let mut builder = agent.get(&request.url);
            for (name, value) in &request.headers {
                builder = builder.header(*name, value);
            }
            builder.call()
        }
        Method::Post => {
            let mut builder = agent.post(&request.url);
            for (name, value) in &request.headers {
                builder = builder.header(*name, value);
            }
            builder.send(request.body.as_deref().unwrap_or_default())
        }
    };
    let mut response = result.ok()?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .read_to_string()
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    Some(Response { status, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-22T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn grant(overrides: Value) -> Value {
        let mut grant = json!({ "id": "grant_a", "resets_left": 1, "usable_now": true });
        for (key, value) in overrides.as_object().unwrap() {
            grant[key] = value.clone();
        }
        grant
    }

    #[test]
    fn counts_usable_grants_and_keeps_the_next_one() {
        let block = json!({
            "eligible": true,
            "next_grant_id": "grant_a",
            "grants": [
                grant(json!({ "resets_left": 2, "ends_at": "2026-10-01T00:00:00Z" })),
                grant(json!({ "id": "later", "ends_at": "2026-12-01T00:00:00.500+02:00" })),
                grant(json!({ "id": "paused", "paused": true })),
                grant(json!({ "id": "expired", "ends_at": "2026-09-01T00:00:00Z" })),
                grant(json!({ "id": "garbled", "ends_at": "not a date" })),
                grant(json!({ "id": "date_only", "ends_at": "2026-10-01" })),
                grant(json!({ "id": "impossible", "ends_at": "2027-02-30T00:00:00Z" })),
                grant(json!({ "id": "empty", "ends_at": "" })),
                grant(json!({ "id": "Not Valid" })),
                grant(json!({ "id": "grant_b", "resets_left": 3, "usable_now": false })),
            ],
        });

        assert_eq!(
            banked_resets(&block, now()),
            Some(BankedResets {
                count: 3,
                next_grant_id: Some("grant_a".into()),
                expires_at: vec!["2026-10-01T00:00:00Z".into(), "2026-11-30T22:00:00Z".into()],
                grants: vec![BankedGrant {
                    id: "grant_a".into(),
                    kind: ResetKind::Full,
                    resets: 3,
                    expires_at: Some("2026-10-01T00:00:00Z".into()),
                }],
            })
        );
    }

    #[test]
    fn groups_grants_by_what_they_refill_spending_the_next_one_of_each_kind() {
        let block = json!({
            "eligible": true,
            "next_grant_id": "fable_late",
            "grants": [
                grant(json!({ "id": "full", "type": "full", "ends_at": "2026-10-01T00:00:00Z" })),
                grant(json!({
                    "id": "fable_soon",
                    "type": "full",
                    "gives_back_weekly": true,
                    "ends_at": "2026-10-01T00:00:00Z",
                })),
                grant(json!({
                    "id": "fable_late",
                    "gives_back_weekly": true,
                    "ends_at": "2026-12-01T00:00:00Z",
                })),
                grant(json!({ "id": "session", "clears": ["five_hour"] })),
                grant(json!({ "id": "weekly", "clears": ["seven_day"] })),
                grant(json!({ "id": "spent", "type": "session", "resets_left": 0 })),
            ],
        });

        let banked = banked_resets(&block, now()).unwrap();
        let kinds: Vec<_> = banked
            .grants
            .iter()
            .map(|grant| (grant.kind, grant.id.as_str(), grant.resets))
            .collect();

        assert_eq!(
            kinds,
            [
                (ResetKind::Session, "session", 1),
                (ResetKind::Weekly, "weekly", 1),
                (ResetKind::Fable, "fable_late", 2),
                (ResetKind::Full, "full", 1),
            ]
        );
        assert_eq!(banked.count, 5);
        assert_eq!(
            banked.grants[2].expires_at.as_deref(),
            Some("2026-12-01T00:00:00Z")
        );
    }

    #[test]
    fn nothing_is_spendable_without_a_usable_next_grant() {
        let unusable = json!({
            "eligible": true,
            "next_grant_id": "grant_a",
            "grants": [grant(json!({ "usable_now": false }))],
        });
        let no_next = json!({ "eligible": true, "grants": [grant(json!({}))] });

        assert_eq!(
            banked_resets(&unusable, now()),
            Some(BankedResets::default())
        );
        assert_eq!(
            banked_resets(&no_next, now()),
            Some(BankedResets::default())
        );
    }

    #[test]
    fn an_ineligible_plan_banks_nothing() {
        let ineligible = json!({ "eligible": false, "grants": [grant(json!({}))] });

        assert_eq!(banked_resets(&ineligible, now()), None);
        assert_eq!(banked_resets(&Value::Null, now()), None);
    }

    #[test]
    fn reads_with_the_cli_s_request() {
        let request = read_request("oauth-token", "2.1.0");

        assert_eq!(request.method, Method::Get);
        assert_eq!(
            request.url,
            "https://api.anthropic.com/api/oauth/usage?cedar_ember=1&skip_spend=1"
        );
        assert_eq!(
            request.headers,
            [
                ("authorization", "Bearer oauth-token".to_owned()),
                ("anthropic-beta", "oauth-2025-04-20".to_owned()),
                ("user-agent", "claude-cli/2.1.0 (external, cli)".to_owned()),
            ]
        );
        assert_eq!(request.body, None);
    }

    #[test]
    fn spends_the_grant_for_the_organization() {
        let request = spend_request("oauth-token", "org 1", "2.1.0", "grant_a", "r-1");

        assert_eq!(request.method, Method::Post);
        assert_eq!(
            request.url,
            "https://api.anthropic.com/api/organizations/org%201/reset_rate_limits"
        );
        assert!(
            request
                .headers
                .contains(&("authorization", "Bearer oauth-token".to_owned()))
        );
        assert!(
            request
                .headers
                .contains(&("content-type", "application/json".to_owned()))
        );
        let body: Value = serde_json::from_str(request.body.as_deref().unwrap()).unwrap();
        assert_eq!(
            body,
            json!({ "program": "cedar_ember", "grant_id": "grant_a", "request_id": "r-1" })
        );
    }

    #[test]
    fn each_answer_maps_to_an_outcome_or_a_failure() {
        let answer = |result: &str| spend_outcome(200, &json!({ "result": result }));
        let failed = |reason| BankedResetSpend::Failed { reason };

        assert_eq!(answer("reset"), BankedResetSpend::Reset);
        assert_eq!(answer("not_limited"), BankedResetSpend::NothingToReset);
        assert_eq!(answer("already_used"), BankedResetSpend::AlreadyUsed);
        assert_eq!(answer("ineligible"), BankedResetSpend::NoneLeft);
        assert_eq!(answer("cooldown"), failed(BankedResetFailure::CoolingDown));
        assert_eq!(
            answer("unavailable"),
            failed(BankedResetFailure::Unconfirmed)
        );
        assert_eq!(
            answer("surprise"),
            failed(BankedResetFailure::RequestFailed)
        );
        assert_eq!(
            spend_outcome(429, &Value::Null),
            failed(BankedResetFailure::RateLimited)
        );
        assert_eq!(
            spend_outcome(401, &Value::Null),
            failed(BankedResetFailure::SignedOut)
        );
        assert_eq!(
            spend_outcome(500, &Value::Null),
            failed(BankedResetFailure::RequestFailed)
        );
    }

    #[test]
    fn malformed_ids_are_never_sent() {
        assert_eq!(
            spend("2.1.0", "Not Valid", "r-1"),
            BankedResetSpend::NoneLeft
        );
        assert_eq!(
            spend("2.1.0", "grant_a", "bad id"),
            BankedResetSpend::NoneLeft
        );
    }
}
