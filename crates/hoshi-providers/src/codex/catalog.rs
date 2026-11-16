//! What Codex's app-server says of its models, the account signed in and the plan's limits.

use super::PROVIDER;
use super::items::effort;
use hoshi_core::{
    BankedResets, ModelInfo, ProviderAccount, ProviderModels, ProviderPlan, UsageLimits,
    UsageWindow, UsageWindowKind,
};
use serde_json::Value;

const SESSION_MINUTES: u32 = 5 * 60;
const WEEK_MINUTES: u32 = 7 * 24 * 60;
const MONTH_MINUTES: u32 = 30 * 24 * 60;
/// The effort that also hands work to subagents on its own, as hoshi's ultracode does.
pub(crate) const ULTRA_EFFORT: &str = "ultra";
/// The service tiers that make a model faster.
const FAST_TIERS: [&str; 2] = ["fast", "priority"];

/// The models `model/list` offered, hidden ones left out.
pub(crate) fn catalog(models: &[Value]) -> ProviderModels {
    let models: Vec<&Value> = models
        .iter()
        .filter(|model| model["hidden"] != true)
        .collect();
    let default_model = models
        .iter()
        .find(|model| model["isDefault"] == true)
        .or(models.first())
        .and_then(|model| slug(model));
    ProviderModels {
        provider: PROVIDER.into(),
        default_model,
        models: models.into_iter().filter_map(model_info).collect(),
        ..ProviderModels::default()
    }
}

fn slug(model: &Value) -> Option<String> {
    model["model"]
        .as_str()
        .or_else(|| model["id"].as_str())
        .filter(|slug| !slug.is_empty())
        .map(str::to_owned)
}

fn model_info(model: &Value) -> Option<ModelInfo> {
    let slug = slug(model)?;
    let name = model["displayName"]
        .as_str()
        .filter(|name| !name.is_empty())
        .unwrap_or(&slug)
        .to_owned();
    let supported: Vec<&str> = model["supportedReasoningEfforts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|option| option["reasoningEffort"].as_str())
        .collect();
    Some(ModelInfo {
        short_name: name.clone(),
        name,
        slug,
        legacy: false,
        is_new: false,
        update_required: false,
        efforts: supported.iter().filter_map(|value| effort(value)).collect(),
        default_effort: model["defaultReasoningEffort"].as_str().and_then(effort),
        fast_mode: fast_tier(model).is_some(),
        ultracode: supported.contains(&ULTRA_EFFORT),
    })
}

/// The service tier a fast run of `model` asks for, when it has one.
pub(crate) fn fast_tier(model: &Value) -> Option<String> {
    let tiers = model["serviceTiers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tier| tier["id"].as_str())
        .chain(
            model["additionalSpeedTiers"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str),
        );
    tiers
        .into_iter()
        .find(|tier| FAST_TIERS.contains(tier))
        .map(str::to_owned)
}

/// The plan's limits from `account/rateLimits/read`.
pub(crate) fn usage_limits(response: &Value) -> UsageLimits {
    let snapshot = match &response["rateLimitsByLimitId"]["codex"] {
        Value::Null => &response["rateLimits"],
        codex => codex,
    };
    UsageLimits {
        windows: windows(snapshot),
        banked_resets: banked_resets(&response["rateLimitResetCredits"]),
        account: None,
    }
}

/// The windows a mid-turn `account/rateLimits/updated` reports; `None` when it reports none
/// of the plan's own.
pub(crate) fn usage_update(params: &Value) -> Option<UsageLimits> {
    let windows = windows(&params["rateLimits"]);
    if windows.is_empty() {
        return None;
    }
    Some(UsageLimits {
        windows,
        banked_resets: None,
        account: None,
    })
}

/// When a plan whose limit was reached can run again: once every window used up resets.
/// `None` when none is used up or one does not tell when it resets.
pub(crate) fn resets_at(limits: &UsageLimits) -> Option<String> {
    let mut used_up = limits
        .windows
        .iter()
        .filter(|window| window.used_percent >= 100)
        .peekable();
    used_up.peek()?;
    used_up
        .map(|window| {
            let at = window.resets_at.as_deref()?;
            chrono::DateTime::parse_from_rfc3339(at)
                .ok()
                .map(|time| (time, at))
        })
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .max_by_key(|(time, _)| *time)
        .map(|(_, at)| at.to_owned())
}

/// The plan's rolling windows. A snapshot of another limit, such as one model's, has none.
/// A month-long window, as free plans have, counts as weekly: hoshi has no longer kind.
fn windows(snapshot: &Value) -> Vec<UsageWindow> {
    if snapshot["limitId"]
        .as_str()
        .is_some_and(|limit| limit != "codex")
    {
        return Vec::new();
    }
    let monthly = matches!(snapshot["planType"].as_str(), Some("free" | "go"));
    let positions = [
        (
            &snapshot["primary"],
            if monthly {
                MONTH_MINUTES
            } else {
                SESSION_MINUTES
            },
        ),
        (&snapshot["secondary"], WEEK_MINUTES),
    ];
    positions
        .into_iter()
        .filter_map(|(window, fallback)| {
            let used = window["usedPercent"].as_f64()?;
            let minutes = window["windowDurationMins"]
                .as_u64()
                .map_or(fallback, |minutes| minutes as u32);
            Some(UsageWindow {
                kind: if minutes >= WEEK_MINUTES {
                    UsageWindowKind::Weekly
                } else {
                    UsageWindowKind::Session
                },
                model: None,
                used_percent: used.clamp(0.0, 100.0).round() as u8,
                window_minutes: minutes,
                resets_at: window["resetsAt"].as_i64().and_then(rfc3339),
            })
        })
        .collect()
}

fn banked_resets(summary: &Value) -> Option<BankedResets> {
    let count = summary["availableCount"].as_u64()?;
    let available: Vec<&Value> = summary["credits"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|credit| credit["status"] == "available")
        .collect();
    let mut expiries: Vec<i64> = available
        .iter()
        .filter_map(|credit| credit["expiresAt"].as_i64())
        .collect();
    expiries.sort_unstable();
    Some(BankedResets {
        count: count as u32,
        next_grant_id: available
            .iter()
            .find_map(|credit| credit["id"].as_str().map(str::to_owned)),
        expires_at: expiries.into_iter().filter_map(rfc3339).collect(),
    })
}

fn rfc3339(seconds: i64) -> Option<String> {
    (seconds > 0)
        .then(|| chrono::DateTime::from_timestamp(seconds, 0))
        .flatten()
        .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

/// Who `account/read` says is signed in; `None` when no one is.
pub(crate) fn account(response: &Value) -> Option<ProviderAccount> {
    let account = &response["account"];
    if account.is_null() {
        return None;
    }
    Some(ProviderAccount {
        email: account["email"].as_str().map(str::to_owned),
        plan: match account["planType"].as_str() {
            Some("pro" | "prolite" | "promax") => Some(ProviderPlan::Pro),
            Some("team" | "business" | "self_serve_business_prolite") => Some(ProviderPlan::Team),
            Some(plan) if plan.starts_with("enterprise") || plan == "ent26" => {
                Some(ProviderPlan::Enterprise)
            }
            _ => None,
        },
    })
}

/// Whether `account/read` says the CLI needs a login before it can run.
pub(crate) fn signed_out(response: &Value) -> bool {
    response["account"].is_null() && response["requiresOpenaiAuth"] == true
}

#[cfg(test)]
mod tests {
    use super::*;
    use hoshi_core::Effort;
    use serde_json::json;

    #[test]
    fn the_model_list_becomes_the_catalog() {
        let response: Value = serde_json::from_str(r#"{"data":[{"id":"gpt-6-luna","model":"gpt-6-luna","displayName":"GPT-6-Luna","description":"Fast and affordable model for easier tasks.","hidden":false,"supportedReasoningEfforts":[{"reasoningEffort":"low","description":"Fast"},{"reasoningEffort":"medium","description":"Balanced"},{"reasoningEffort":"xhigh","description":"Extra high"},{"reasoningEffort":"max","description":"Maximum"}],"defaultReasoningEffort":"medium","inputModalities":["text","image"],"additionalSpeedTiers":[],"serviceTiers":[],"defaultServiceTier":null,"isDefault":true},{"id":"gpt-5.6-terra","model":"gpt-5.6-terra","displayName":"GPT-5.6-Terra","hidden":false,"supportedReasoningEfforts":[{"reasoningEffort":"low","description":""},{"reasoningEffort":"ultra","description":"Maximum reasoning with automatic task delegation"}],"defaultReasoningEffort":"low","serviceTiers":[{"id":"priority","name":"Fast","description":""}],"isDefault":false},{"id":"codex-internal","model":"codex-internal","displayName":"Internal","hidden":true,"supportedReasoningEfforts":[],"defaultReasoningEffort":"low","isDefault":false}],"nextCursor":null}"#).unwrap();

        let catalog = catalog(response["data"].as_array().unwrap());

        assert_eq!(catalog.provider, "codex");
        assert_eq!(catalog.default_model.as_deref(), Some("gpt-6-luna"));
        assert_eq!(catalog.models.len(), 2);
        let luna = &catalog.models[0];
        assert_eq!(luna.name, "GPT-6-Luna");
        assert_eq!(
            luna.efforts,
            [Effort::Low, Effort::Medium, Effort::Xhigh, Effort::Max]
        );
        assert_eq!(luna.default_effort, Some(Effort::Medium));
        assert!(!luna.fast_mode && !luna.ultracode);
        let terra = &catalog.models[1];
        assert_eq!(terra.efforts, [Effort::Low]);
        assert!(terra.fast_mode && terra.ultracode);
        assert_eq!(fast_tier(&response["data"][1]).as_deref(), Some("priority"));
    }

    #[test]
    fn a_free_plans_month_and_a_paid_plans_session_and_week() {
        let free: Value = serde_json::from_str(r#"{"ordinaryUsageAllowed":true,"rateLimits":{"limitId":"codex","primary":{"usedPercent":1,"windowDurationMins":43200,"resetsAt":1795561547},"secondary":null,"planType":"free"},"rateLimitsByLimitId":{"codex":{"limitId":"codex","limitName":null,"primary":{"usedPercent":1,"windowDurationMins":43200,"resetsAt":1795561547},"secondary":null,"credits":{"hasCredits":false,"unlimited":false,"balance":null},"planType":"free","rateLimitReachedType":null}},"rateLimitResetCredits":{"availableCount":0,"credits":[]},"rateLimitUpsell":null}"#).unwrap();
        let paid = json!({
            "rateLimits": {
                "limitId": "codex",
                "primary": { "usedPercent": 37.6, "resetsAt": 1_795_561_547 },
                "secondary": { "usedPercent": 120, "windowDurationMins": 10_080 },
                "planType": "plus",
            },
            "rateLimitResetCredits": {
                "availableCount": 2,
                "credits": [
                    { "id": "credit_b", "status": "available", "expiresAt": 1_795_600_000, "grantedAt": 1, "resetType": "session" },
                    { "id": "credit_a", "status": "redeemed", "expiresAt": 1_795_500_000, "grantedAt": 1, "resetType": "session" },
                ],
            },
        });

        let free = usage_limits(&free);
        let paid = usage_limits(&paid);

        assert_eq!(
            free.windows,
            [UsageWindow {
                kind: UsageWindowKind::Weekly,
                model: None,
                used_percent: 1,
                window_minutes: MONTH_MINUTES,
                resets_at: Some("2026-11-24T23:05:47Z".into()),
            }]
        );
        assert_eq!(
            free.banked_resets,
            Some(BankedResets {
                count: 0,
                next_grant_id: None,
                expires_at: Vec::new(),
            })
        );
        assert_eq!(paid.windows[0].kind, UsageWindowKind::Session);
        assert_eq!(paid.windows[0].window_minutes, SESSION_MINUTES);
        assert_eq!(paid.windows[0].used_percent, 38);
        assert_eq!(paid.windows[1].kind, UsageWindowKind::Weekly);
        assert_eq!(paid.windows[1].used_percent, 100);
        let banked = paid.banked_resets.unwrap();
        assert_eq!(banked.count, 2);
        assert_eq!(banked.next_grant_id.as_deref(), Some("credit_b"));
        assert_eq!(banked.expires_at.len(), 1);
    }

    #[test]
    fn another_limits_update_leaves_the_plans_windows_alone() {
        let own: Value = serde_json::from_str(r#"{"rateLimits":{"limitId":"codex","limitName":null,"normalModelSlug":null,"primary":{"usedPercent":1,"windowDurationMins":43200,"resetsAt":1795561546},"secondary":null,"credits":{"hasCredits":false,"unlimited":false,"balance":null},"individualLimit":null,"spendControlReached":null,"planType":"free","rateLimitReachedType":null}}"#).unwrap();
        let other =
            json!({ "rateLimits": { "limitId": "gpt-6-astra", "primary": { "usedPercent": 50 } } });

        assert_eq!(usage_update(&own).unwrap().windows.len(), 1);
        assert_eq!(usage_update(&other), None);
    }

    #[test]
    fn the_account_names_its_email_and_plan() {
        let signed_in: Value = serde_json::from_str(r#"{"account":{"type":"chatgpt","email":"nxtvoid@example.com","planType":"pro","workspaceName":null},"requiresOpenaiAuth":true}"#).unwrap();
        let signed_out = json!({ "account": null, "requiresOpenaiAuth": true });
        let api_key = json!({ "account": { "type": "apiKey" }, "requiresOpenaiAuth": true });

        assert_eq!(
            account(&signed_in),
            Some(ProviderAccount {
                email: Some("nxtvoid@example.com".into()),
                plan: Some(ProviderPlan::Pro),
            })
        );
        assert!(!super::signed_out(&signed_in));
        assert!(super::signed_out(&signed_out));
        assert_eq!(account(&signed_out), None);
        assert_eq!(
            account(&api_key),
            Some(ProviderAccount {
                email: None,
                plan: None
            })
        );
    }
}
