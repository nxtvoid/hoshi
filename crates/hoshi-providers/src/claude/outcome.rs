//! How a run ended, read from its `result` and from what the frames before it said: Claude
//! Code reports failures as data, often under `subtype: "success"`.

use super::usage::epoch_time;
use crate::version::parse_version;
use hoshi_core::{ProviderOutput, TurnFailure, TurnOutcome};
use serde_json::Value;
use std::collections::HashMap;

const MISSING_SESSION: &str = "No conversation found with session ID";
const DIAGNOSTIC: &str = "[ede_diagnostic]";
const LOCAL_STDOUT_OPEN: &str = "<local-command-stdout>";
const LOCAL_STDOUT_CLOSE: &str = "</local-command-stdout>";

/// What the turn's frames told before its result.
#[derive(Debug, Default)]
pub(super) struct Signals {
    /// Plan limits that refused the agent's calls, by limit type, with when each resets.
    rejected: HashMap<String, Option<String>>,
    /// The agent's latest message was refused for a rate limit: the limit's type and when it
    /// resets, as far as it told.
    assistant_limit: Option<(Option<String>, Option<String>)>,
    signed_out: bool,
    /// The CLI's own version, as the session reported it.
    version: Option<String>,
}

impl Signals {
    pub fn session_started(&mut self, init: &Value) {
        if let Some(version) = init["claude_code_version"].as_str() {
            self.version = Some(version.to_owned());
        }
    }

    pub fn rate_limit_event(&mut self, info: &Value) {
        let kind = info["rateLimitType"]
            .as_str()
            .unwrap_or("unknown")
            .to_owned();
        let overage = overage_allowed(info);
        if info["status"] == "rejected" && !overage {
            self.rejected.insert(kind, reset_time(&info["resetsAt"]));
        } else if matches!(info["status"].as_str(), Some("allowed" | "allowed_warning")) || overage
        {
            self.rejected.remove(&kind);
        }
    }

    /// A message of the agent's own, which tells when the API refused it.
    pub fn assistant(&mut self, frame: &Value) {
        self.assistant_limit = (frame["error"] == "rate_limit").then(|| {
            let info = &frame["api_error_params"]["rate_limit_info"];
            if info["status"] == "rejected" && !overage_allowed(info) {
                let kind = info["rateLimitType"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_owned();
                (Some(kind), reset_time(&info["resetsAt"]))
            } else {
                (None, None)
            }
        });
        if frame["error"] == "authentication_failed" {
            self.signed_out = true;
        }
    }

    /// Forgets what the turn that ended told.
    pub fn turn_ended(&mut self) {
        let version = self.version.take();
        *self = Self {
            version,
            ..Self::default()
        };
    }

    fn limited(&self) -> bool {
        !self.rejected.is_empty() || self.assistant_limit.is_some()
    }

    /// When every limit that refused the turn resets, if each told.
    fn resets_at(&self) -> Option<String> {
        let mut times: Vec<&Option<String>> = self.rejected.values().collect();
        if let Some((Some(kind), reset)) = &self.assistant_limit
            && !self.rejected.contains_key(kind)
        {
            times.push(reset);
        }
        if times.is_empty() || times.iter().any(|time| time.is_none()) {
            return None;
        }
        times.into_iter().flatten().max().cloned()
    }
}

fn overage_allowed(info: &Value) -> bool {
    matches!(
        info["overageStatus"].as_str(),
        Some("allowed" | "allowed_warning")
    ) || info["isUsingOverage"] == true
        || info["overageInUse"] == true
}

fn reset_time(resets_at: &Value) -> Option<String> {
    resets_at.as_i64().and_then(epoch_time)
}

/// Terminal reasons that fail a run whatever its subtype says.
fn fails_run(terminal_reason: &str) -> bool {
    matches!(
        terminal_reason,
        "api_error"
            | "malformed_tool_use_exhausted"
            | "budget_exhausted"
            | "structured_output_retry_exhausted"
            | "tool_deferred_unavailable"
            | "turn_setup_failed"
            | "blocking_limit"
            | "rapid_refill_breaker"
            | "prompt_too_long"
            | "image_error"
            | "model_error"
    )
}

/// How the run `result` ended, given what the turn told before it; `now` in seconds since
/// the epoch, for limits that say how long until they reset.
pub(super) fn outcome(result: &Value, signals: &Signals, now: i64) -> TurnOutcome {
    let terminal = result["terminal_reason"].as_str();
    if matches!(terminal, Some("aborted_tools" | "aborted_streaming")) {
        return TurnOutcome::Interrupted;
    }
    let subtype = result["subtype"].as_str().unwrap_or("error");
    let success = subtype == "success";
    let status = result["api_error_status"].as_u64();
    let errors: Vec<&str> = result["errors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    if !success {
        let said = errors.join("\n").to_lowercase();
        if said.contains("interrupt") || said.contains("cancel") {
            return TurnOutcome::Interrupted;
        }
    }
    let failed = !success
        || result["is_error"] == true
        || matches!(status, Some(429 | 529))
        || terminal.is_some_and(fails_run);
    if !failed {
        return TurnOutcome::Completed;
    }
    let text = result["result"].as_str().unwrap_or_default().trim();
    let listed = errors
        .iter()
        .copied()
        .find(|error| !error.starts_with(DIAGNOSTIC) && !error.trim().is_empty());
    let message = listed.or((!text.is_empty()).then_some(text));
    let detail = match message {
        Some(message) => format!("{subtype}: {message}"),
        None => format!("{subtype}: {}", terminal.unwrap_or("error")),
    };
    let reason = if errors
        .iter()
        .chain([&text])
        .any(|said| said.contains(MISSING_SESSION))
    {
        TurnFailure::ResumeFailed { detail }
    } else if signals.signed_out || status == Some(401) {
        TurnFailure::AuthenticationFailed { detail }
    } else if let Some(resets_at) = usage_limit(result, signals, now) {
        TurnFailure::UsageLimited { detail, resets_at }
    } else if status == Some(529) {
        TurnFailure::Overloaded { detail }
    } else if matches!(terminal, Some("prompt_too_long" | "blocking_limit")) {
        TurnFailure::ContextFull { detail }
    } else if let Some((reported, own)) = rewritten_version(result, signals) {
        TurnFailure::ProviderError {
            detail: format!(
                "{detail} (the API endpoint reported Claude Code {reported}, but the session runs {own}: something between them rewrote the version)"
            ),
        }
    } else {
        TurnFailure::ProviderError { detail }
    };
    TurnOutcome::Failed { reason }
}

/// `Some` when the run failed on a usage limit, holding when it resets if that is known.
fn usage_limit(result: &Value, signals: &Signals, now: i64) -> Option<Option<String>> {
    let success = result["subtype"] == "success";
    let status = result["api_error_status"].as_u64();
    let terminal = result["terminal_reason"].as_str();
    let text = result["result"].as_str().unwrap_or_default();
    let limited = signals.limited()
        && (!success || status.is_none() || status == Some(429))
        && matches!(terminal, None | Some("api_error" | "blocking_limit"));
    let gateway = !limited
        && success
        && result["is_error"] == true
        && terminal == Some("api_error")
        && status == Some(503)
        && text.starts_with("API Error: 503 ")
        && text.contains("[429]")
        && text.contains("\"rate_limit_error\"");
    let gateway_reset = || {
        (text.starts_with("API Error:"))
            .then(|| reset_after(text, now))
            .flatten()
    };
    if gateway {
        return gateway_reset().map(Some);
    }
    if !limited && !(success && status == Some(429)) {
        return None;
    }
    Some(
        signals
            .resets_at()
            .or_else(|| (status == Some(429)).then(gateway_reset).flatten()),
    )
}

/// When a gateway's `(reset after 1h 2m 3s)` ends, counted from `now`.
fn reset_after(text: &str, now: i64) -> Option<String> {
    let (_, rest) = text.split_once("(reset after ")?;
    let (span, _) = rest.split_once(')')?;
    let mut seconds = 0;
    let mut number = String::new();
    for char in span.chars() {
        match char {
            '0'..='9' => number.push(char),
            'h' | 'm' | 's' => {
                let value: i64 = std::mem::take(&mut number).parse().ok()?;
                seconds += value
                    * match char {
                        'h' => 3_600,
                        'm' => 60,
                        _ => 1,
                    };
            }
            ' ' => {}
            _ => return None,
        }
    }
    (seconds > 0 && number.is_empty())
        .then(|| epoch_time(now + seconds))
        .flatten()
}

/// The version a gateway put in place of the session's own, when the session itself meets
/// what the model requires: (reported, own).
fn rewritten_version(result: &Value, signals: &Signals) -> Option<(String, String)> {
    if result["subtype"] != "success" || result["terminal_reason"] != "api_error" {
        return None;
    }
    let text = result["result"].as_str()?.trim();
    let rest = text.strip_prefix("API Error: 400 Claude Code ")?;
    let (reported, rest) = rest.split_once(" does not support this model; version ")?;
    let (required, _) = rest.split_once(" or newer is required.")?;
    let own = signals.version.as_deref()?;
    let (reported_version, own_version, required_version) = (
        parse_version(reported)?,
        parse_version(own)?,
        parse_version(required)?,
    );
    (reported_version < own_version && own_version >= required_version)
        .then(|| (reported.to_owned(), own.to_owned()))
}

/// A `system/api_retry` frame: the CLI failed to reach the model and tries again.
pub(super) fn retrying(frame: &Value) -> ProviderOutput {
    let count = |key: &str| frame[key].as_f64().map(|value| value.max(0.0) as u64);
    ProviderOutput::Retrying {
        attempt: count("attempt").unwrap_or(1).max(1) as u32,
        max_attempts: count("max_retries").map(|max| max.max(1) as u32),
        retry_in_ms: count("retry_delay_ms"),
        error: frame["error"]
            .as_str()
            .filter(|error| !error.is_empty())
            .map(str::to_owned),
    }
}

/// Whether the CLI's own error output says the session to resume doesn't exist.
pub(super) fn missing_session(stderr: &str) -> bool {
    stderr.contains(MISSING_SESSION)
}

/// What a local command such as `/cost` printed, from the user frame the CLI replays it in.
pub(super) fn local_command_output(frame: &Value) -> Option<String> {
    if frame["isReplay"] != true {
        return None;
    }
    let content = frame["message"]["content"].as_str()?.trim();
    let inner = content
        .strip_prefix(LOCAL_STDOUT_OPEN)?
        .strip_suffix(LOCAL_STDOUT_CLOSE)?
        .trim();
    (!inner.is_empty()).then(|| inner.to_owned())
}

/// What a completed run that streamed no text of its own answered, from its result, when
/// Claude Code answered it without the model, as a local command such as `/cost` is: its
/// text, or what the command printed. `/compact` reports through its compaction instead.
pub(super) fn reply_text(result: &Value, local_output: Option<&str>) -> Option<String> {
    let local = result["local_command"].is_string() || result["num_turns"] == 0;
    if result["subtype"] != "success" || !local || result["local_command"] == "compact" {
        return None;
    }
    let text = result["result"].as_str().unwrap_or_default().trim();
    if !text.is_empty() {
        return Some(text.to_owned());
    }
    local_output.map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_791_800_000;

    fn failure(result: Value, signals: &Signals) -> TurnFailure {
        match outcome(&result, signals, NOW) {
            TurnOutcome::Failed { reason } => reason,
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn a_rejected_rate_limit_fails_the_turn_with_when_it_resets() {
        let mut signals = Signals::default();
        signals.rate_limit_event(&json!({ "status": "rejected", "rateLimitType": "seven_day",
            "resetsAt": 1_791_807_600 }));
        signals.assistant(&json!({ "type": "assistant", "error": "rate_limit",
            "api_error_params": { "rate_limit_info": { "status": "rejected",
                "rateLimitType": "five_hour", "resetsAt": 1_791_804_000 } } }));
        let result = json!({ "type": "result", "subtype": "success", "is_error": true,
            "api_error_status": 429, "terminal_reason": "api_error",
            "result": "Usage limit reached." });

        assert_eq!(
            failure(result, &signals),
            TurnFailure::UsageLimited {
                detail: "success: Usage limit reached.".into(),
                resets_at: Some("2026-10-12T12:20:00Z".into()),
            }
        );
    }

    #[test]
    fn a_blocked_message_alone_is_a_usage_limit() {
        let mut signals = Signals::default();
        signals.assistant(&json!({ "error": "rate_limit", "api_error_params": {
            "rate_limit_info": { "status": "rejected", "rateLimitType": "five_hour",
                "resetsAt": 1_791_804_000 } } }));
        let result = json!({ "subtype": "success", "is_error": true,
            "terminal_reason": "api_error", "result": "You've hit your limit" });

        assert!(matches!(
            failure(result, &signals),
            TurnFailure::UsageLimited { resets_at: Some(at), .. } if at == "2026-10-12T11:20:00Z"
        ));
    }

    #[test]
    fn a_limit_that_allows_again_or_overage_no_longer_counts() {
        let mut signals = Signals::default();
        signals.rate_limit_event(&json!({ "status": "rejected", "rateLimitType": "five_hour" }));
        signals.rate_limit_event(&json!({ "status": "allowed", "rateLimitType": "five_hour" }));
        signals.rate_limit_event(&json!({ "status": "rejected", "rateLimitType": "seven_day",
            "overageStatus": "allowed" }));
        let result = json!({ "subtype": "success", "is_error": true,
            "terminal_reason": "api_error", "result": "API Error: 500" });

        assert!(matches!(
            failure(result, &signals),
            TurnFailure::ProviderError { .. }
        ));
    }

    #[test]
    fn a_429_without_a_limit_event_is_a_limit_of_unknown_reset() {
        let result = json!({ "subtype": "success", "is_error": true, "api_error_status": 429,
            "terminal_reason": "api_error", "result": "API Error: 429 rate limited" });
        assert!(matches!(
            failure(result, &Signals::default()),
            TurnFailure::UsageLimited {
                resets_at: None,
                ..
            }
        ));

        let gateway = json!({ "subtype": "success", "is_error": true, "api_error_status": 429,
            "terminal_reason": "api_error",
            "result": "API Error: 429 slow down (reset after 1h 2m 5s)" });
        assert!(matches!(
            failure(gateway, &Signals::default()),
            TurnFailure::UsageLimited { resets_at: Some(at), .. } if at == "2026-10-12T11:15:25Z"
        ));
    }

    #[test]
    fn a_gateway_cooldown_behind_a_503_is_a_usage_limit() {
        let result = json!({ "subtype": "success", "is_error": true, "api_error_status": 503,
            "terminal_reason": "api_error",
            "result": "API Error: 503 [gw] [429]: {\"type\":\"error\",\"error\":{\"type\":\"rate_limit_error\",\"message\":\"slow\"}} (reset after 30s)" });

        assert!(matches!(
            failure(result, &Signals::default()),
            TurnFailure::UsageLimited { resets_at: Some(at), .. } if at == "2026-10-12T10:13:50Z"
        ));
    }

    #[test]
    fn an_overloaded_api_and_a_signed_out_account_have_their_own_failures() {
        let overloaded = json!({ "subtype": "success", "is_error": false,
            "api_error_status": 529, "result": "API Error: 529 Overloaded" });
        assert!(matches!(
            failure(overloaded, &Signals::default()),
            TurnFailure::Overloaded { .. }
        ));

        let mut signals = Signals::default();
        signals.assistant(&json!({ "error": "authentication_failed" }));
        let unauthorized = json!({ "subtype": "success", "is_error": true,
            "result": "Failed to authenticate." });
        assert!(matches!(
            failure(unauthorized.clone(), &signals),
            TurnFailure::AuthenticationFailed { .. }
        ));
        let by_status = json!({ "subtype": "success", "is_error": true, "api_error_status": 401,
            "terminal_reason": "completed", "result": "Failed to authenticate. API Error: 401" });
        assert!(matches!(
            failure(by_status, &Signals::default()),
            TurnFailure::AuthenticationFailed { .. }
        ));
    }

    #[test]
    fn a_conversation_too_long_for_the_model_fills_its_context() {
        for reason in ["prompt_too_long", "blocking_limit"] {
            let result = json!({ "subtype": "success", "is_error": false,
                "terminal_reason": reason, "result": "Prompt is too long" });
            assert!(matches!(
                failure(result, &Signals::default()),
                TurnFailure::ContextFull { .. }
            ));
        }
    }

    #[test]
    fn a_run_cut_short_is_interrupted_even_when_it_says_it_succeeded() {
        for reason in ["aborted_streaming", "aborted_tools"] {
            let result = json!({ "subtype": "success", "is_error": false,
                "terminal_reason": reason, "result": "1\n2" });
            assert_eq!(
                outcome(&result, &Signals::default(), NOW),
                TurnOutcome::Interrupted
            );
        }
        let stopped = json!({ "subtype": "error_during_execution", "is_error": true,
            "errors": ["Request was interrupted by the user"] });
        assert_eq!(
            outcome(&stopped, &Signals::default(), NOW),
            TurnOutcome::Interrupted
        );
    }

    #[test]
    fn a_success_with_an_error_status_or_a_failing_reason_failed() {
        let throttled = json!({ "subtype": "success", "is_error": false,
            "api_error_status": 429, "result": "" });
        assert!(matches!(
            outcome(&throttled, &Signals::default(), NOW),
            TurnOutcome::Failed { .. }
        ));
        let gave_up = json!({ "subtype": "success", "is_error": false,
            "terminal_reason": "model_error", "result": "" });
        assert!(matches!(
            outcome(&gave_up, &Signals::default(), NOW),
            TurnOutcome::Failed {
                reason: TurnFailure::ProviderError { .. }
            }
        ));
        let fine = json!({ "subtype": "success", "is_error": false,
            "terminal_reason": "completed", "result": "ok" });
        assert_eq!(
            outcome(&fine, &Signals::default(), NOW),
            TurnOutcome::Completed
        );
    }

    #[test]
    fn a_session_that_no_longer_exists_fails_to_resume() {
        let result = json!({ "type": "result", "subtype": "error_during_execution",
            "is_error": true, "num_turns": 0,
            "errors": ["No conversation found with session ID: 9b0c1d2e-3f40-4a5b-8c6d-7e8f9a0b1c2d"] });

        assert!(matches!(
            failure(result, &Signals::default()),
            TurnFailure::ResumeFailed { detail } if detail.contains("9b0c1d2e")
        ));
        assert!(missing_session(
            "No conversation found with session ID: 9b0c1d2e"
        ));
    }

    #[test]
    fn diagnostics_stay_out_of_the_detail() {
        let result = json!({ "subtype": "error_max_turns", "is_error": true,
            "errors": ["[ede_diagnostic] stream ended", "Reached the maximum number of turns"] });
        assert_eq!(
            failure(result, &Signals::default()),
            TurnFailure::ProviderError {
                detail: "error_max_turns: Reached the maximum number of turns".into()
            }
        );
    }

    #[test]
    fn a_version_a_gateway_rewrote_is_told_apart() {
        let mut signals = Signals::default();
        signals.session_started(&json!({ "claude_code_version": "2.1.325" }));
        let result = json!({ "subtype": "success", "is_error": true,
            "terminal_reason": "api_error",
            "result": "API Error: 400 Claude Code 2.0.1 does not support this model; version 2.1.0 or newer is required." });

        assert!(matches!(
            failure(result.clone(), &signals),
            TurnFailure::ProviderError { detail } if detail.contains("rewrote")
        ));
        signals.turn_ended();
        assert!(matches!(
            failure(result, &signals),
            TurnFailure::ProviderError { detail } if detail.contains("rewrote")
        ));
    }

    #[test]
    fn signals_are_forgotten_once_the_turn_ends() {
        let mut signals = Signals::default();
        signals.assistant(&json!({ "error": "authentication_failed" }));
        signals.rate_limit_event(&json!({ "status": "rejected", "rateLimitType": "five_hour" }));
        signals.turn_ended();
        let result = json!({ "subtype": "success", "is_error": true, "result": "boom" });

        assert!(matches!(
            failure(result, &signals),
            TurnFailure::ProviderError { .. }
        ));
    }

    #[test]
    fn an_api_retry_tells_its_attempt_and_delay() {
        let frame = json!({ "type": "system", "subtype": "api_retry", "attempt": 2,
            "max_retries": 10, "retry_delay_ms": 1216, "error_status": null, "error": "unknown" });

        assert_eq!(
            retrying(&frame),
            ProviderOutput::Retrying {
                attempt: 2,
                max_attempts: Some(10),
                retry_in_ms: Some(1216),
                error: Some("unknown".into()),
            }
        );
    }

    #[test]
    fn a_result_answers_with_its_text_or_what_its_command_printed() {
        let cost = json!({ "subtype": "success", "local_command": "cost",
            "result": "Total cost: $0.01" });
        assert_eq!(
            reply_text(&cost, None).as_deref(),
            Some("Total cost: $0.01")
        );
        let printed_only =
            json!({ "subtype": "success", "local_command": "context", "result": "" });
        assert_eq!(
            reply_text(&printed_only, Some("Context: 3%")).as_deref(),
            Some("Context: 3%")
        );
        let plain = json!({ "subtype": "success", "result": "" });
        assert_eq!(reply_text(&plain, Some("Set model to haiku")), None);
        let answered = json!({ "subtype": "success", "num_turns": 1, "result": "Done." });
        assert_eq!(reply_text(&answered, None), None);
        let compact = json!({ "subtype": "success", "local_command": "compact",
            "result": "Not enough messages to compact." });
        assert_eq!(reply_text(&compact, None), None);
    }

    #[test]
    fn a_replayed_local_command_s_output_is_read() {
        let frame = json!({ "type": "user", "isReplay": true, "message": { "role": "user",
            "content": "<local-command-stdout>Set model to `haiku`</local-command-stdout>" } });
        assert_eq!(
            local_command_output(&frame).as_deref(),
            Some("Set model to `haiku`")
        );
        let typed = json!({ "type": "user", "message": { "content": "<local-command-stdout>x</local-command-stdout>" } });
        assert_eq!(local_command_output(&typed), None);
    }
}
