//! Whether Claude Code is signed in, read from its `initialize` answer.

use crate::claude_control;
use crate::updater::find_program;
use hoshi_core::ProviderAccountStatus;
use serde_json::{Value, json};
use std::time::Duration;

const REQUEST_ID: &str = "hoshi-account";
const TIMEOUT: Duration = Duration::from_secs(20);

/// Asks a CLI started only for this, from outside any project.
pub(crate) fn probe(binary: &str) -> ProviderAccountStatus {
    if find_program(binary, false).is_none() {
        return ProviderAccountStatus::NotInstalled;
    }
    let request = json!({ "subtype": "initialize" });
    let temp = std::env::temp_dir();
    match claude_control::ask_within(binary, &temp, REQUEST_ID, request, TIMEOUT) {
        Some(init) => account_status(&init),
        None => ProviderAccountStatus::Unknown,
    }
}

/// Signed out only when nothing in `account` points to a way to authenticate.
fn account_status(init: &Value) -> ProviderAccountStatus {
    let account = &init["account"];
    if !account.is_object() {
        return ProviderAccountStatus::Unknown;
    }
    let field = |key: &str| {
        account[key]
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty() && *text != "none")
    };
    let third_party = field("apiProvider").is_some_and(|provider| provider != "firstParty");
    let signed_in = third_party
        || ["tokenSource", "apiKeySource", "email", "subscriptionType"]
            .into_iter()
            .any(|key| field(key).is_some());
    if signed_in {
        ProviderAccountStatus::Ready
    } else {
        ProviderAccountStatus::SignedOut
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(account: &str) -> ProviderAccountStatus {
        let line = format!(
            r#"{{"type":"control_response","response":{{"subtype":"success","request_id":"hoshi-account","response":{{"commands":[],"models":[]{account}}}}}}}"#
        );
        let init = claude_control::response(&line, REQUEST_ID).unwrap();
        account_status(&init)
    }

    #[test]
    fn a_subscription_login_is_ready() {
        assert_eq!(
            status(
                r#","account":{"email":"me@example.com","subscriptionType":"max","tokenSource":"claude.ai","apiProvider":"firstParty"}"#
            ),
            ProviderAccountStatus::Ready
        );
    }

    #[test]
    fn an_api_key_or_another_provider_is_ready() {
        assert_eq!(
            status(r#","account":{"tokenSource":"none","apiKeySource":"ANTHROPIC_API_KEY"}"#),
            ProviderAccountStatus::Ready
        );
        assert_eq!(
            status(r#","account":{"tokenSource":"none","apiProvider":"bedrock"}"#),
            ProviderAccountStatus::Ready
        );
    }

    #[test]
    fn no_token_key_or_account_is_signed_out() {
        assert_eq!(
            status(
                r#","account":{"tokenSource":"none","apiKeySource":"none","apiProvider":"firstParty","email":""}"#
            ),
            ProviderAccountStatus::SignedOut
        );
        assert_eq!(status(r#","account":{}"#), ProviderAccountStatus::SignedOut);
    }

    #[test]
    fn an_answer_without_an_account_is_unknown() {
        assert_eq!(status(""), ProviderAccountStatus::Unknown);
    }
}
