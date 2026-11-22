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
mod tests;
