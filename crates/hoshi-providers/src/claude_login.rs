//! Claude Code's local login files, read and never written.

use hoshi_core::{ProviderAccount, ProviderPlan};
use serde_json::Value;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

fn configured_dir() -> Option<PathBuf> {
    std::env::var("CLAUDE_CONFIG_DIR")
        .ok()
        .map(|dir| dir.trim().to_owned())
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
}

pub(crate) fn config_dir() -> Option<PathBuf> {
    configured_dir().or_else(|| std::env::home_dir().map(|home| home.join(".claude")))
}

/// The account file sits in the configured folder, or in the home folder by default.
fn account_config_path() -> Option<PathBuf> {
    match configured_dir() {
        Some(dir) => Some(dir.join(".claude.json")),
        None => std::env::home_dir().map(|home| home.join(".claude.json")),
    }
}

/// `Ok(None)` when there's no file to read; `Err` when it can't be read or parsed.
fn read_json(path: &Path) -> Result<Option<Value>, ()> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map(Some).map_err(|_| ()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(_) => Err(()),
    }
}

fn non_empty(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// The macOS CLI keeps its login in the keychain, out of reach.
fn credentials(config_dir: &Path) -> Result<Option<Value>, ()> {
    if cfg!(target_os = "macos") {
        return Ok(None);
    }
    read_json(&config_dir.join(".credentials.json"))
}

fn account_config() -> Result<Option<Value>, ()> {
    match account_config_path() {
        Some(path) => read_json(&path),
        None => Ok(None),
    }
}

pub(crate) fn access_token(config_dir: &Path) -> Result<Option<String>, ()> {
    let credentials = credentials(config_dir)?;
    Ok(credentials.and_then(|credentials| non_empty(&credentials["claudeAiOauth"]["accessToken"])))
}

pub(crate) fn organization_id() -> Result<Option<String>, ()> {
    let config = account_config()?;
    Ok(config.and_then(|config| non_empty(&config["oauthAccount"]["organizationUuid"])))
}

/// `None` when neither file names the account.
pub(crate) fn account() -> Option<ProviderAccount> {
    let credentials = config_dir()
        .and_then(|dir| credentials(&dir).ok().flatten())
        .unwrap_or(Value::Null);
    let config = account_config().ok().flatten().unwrap_or(Value::Null);
    account_of(&credentials["claudeAiOauth"], &config["oauthAccount"])
}

/// From the credentials' `claudeAiOauth` and the account file's `oauthAccount`.
fn account_of(oauth: &Value, account: &Value) -> Option<ProviderAccount> {
    let email = non_empty(&account["emailAddress"]).filter(|email| email.contains('@'));
    let plan = plan_of(oauth, account);
    (email.is_some() || plan.is_some()).then_some(ProviderAccount { email, plan })
}

fn plan_of(oauth: &Value, account: &Value) -> Option<ProviderPlan> {
    let subscription = non_empty(&oauth["subscriptionType"]).or_else(|| {
        non_empty(&account["organizationType"])
            .and_then(|kind| kind.strip_prefix("claude_").map(str::to_owned))
    })?;
    let plan = match subscription.to_ascii_lowercase().as_str() {
        "pro" => ProviderPlan::Pro,
        "max" => ProviderPlan::Max,
        "team" => ProviderPlan::Team,
        "enterprise" => ProviderPlan::Enterprise,
        _ => return None,
    };
    if plan != ProviderPlan::Max {
        return Some(plan);
    }
    let tier = non_empty(&oauth["rateLimitTier"])
        .or_else(|| non_empty(&account["organizationRateLimitTier"]))
        .unwrap_or_default();
    Some(if tier.ends_with("_20x") {
        ProviderPlan::Max20x
    } else if tier.ends_with("_5x") {
        ProviderPlan::Max5x
    } else {
        ProviderPlan::Max
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_email_and_the_max_tier() {
        let oauth = json!({
            "accessToken": "token",
            "subscriptionType": "max",
            "rateLimitTier": "default_claude_max_20x",
        });
        let account =
            json!({ "emailAddress": " nxtvoid@example.com ", "organizationType": "claude_max" });

        assert_eq!(
            account_of(&oauth, &account),
            Some(ProviderAccount {
                email: Some("nxtvoid@example.com".into()),
                plan: Some(ProviderPlan::Max20x),
            })
        );
        let five = json!({ "subscriptionType": "max", "rateLimitTier": "default_claude_max_5x" });
        assert_eq!(plan_of(&five, &Value::Null), Some(ProviderPlan::Max5x));
        assert_eq!(
            plan_of(&json!({ "subscriptionType": "max" }), &Value::Null),
            Some(ProviderPlan::Max)
        );
    }

    #[test]
    fn each_subscription_maps_to_a_plan() {
        let plan = |kind: &str| plan_of(&json!({ "subscriptionType": kind }), &Value::Null);

        assert_eq!(plan("pro"), Some(ProviderPlan::Pro));
        assert_eq!(plan("Team"), Some(ProviderPlan::Team));
        assert_eq!(plan("enterprise"), Some(ProviderPlan::Enterprise));
        assert_eq!(plan("free"), None);
        assert_eq!(plan(""), None);
    }

    #[test]
    fn without_credentials_the_account_file_still_names_the_plan() {
        let account = json!({
            "emailAddress": "nxtvoid@example.com",
            "organizationType": "claude_max",
            "organizationRateLimitTier": "default_claude_max_5x",
        });

        assert_eq!(
            account_of(&Value::Null, &account).and_then(|account| account.plan),
            Some(ProviderPlan::Max5x)
        );
        assert_eq!(
            plan_of(&Value::Null, &json!({ "organizationType": "claude_pro" })),
            Some(ProviderPlan::Pro)
        );
    }

    #[test]
    fn an_unknown_login_names_no_account() {
        assert_eq!(account_of(&Value::Null, &Value::Null), None);
        assert_eq!(
            account_of(
                &json!({ "subscriptionType": "free" }),
                &json!({ "emailAddress": "" })
            ),
            None
        );
        assert_eq!(
            account_of(
                &Value::Null,
                &json!({ "emailAddress": "nxtvoid@example.com" })
            ),
            Some(ProviderAccount {
                email: Some("nxtvoid@example.com".into()),
                plan: None,
            })
        );
    }
}
