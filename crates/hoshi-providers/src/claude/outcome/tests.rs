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
    let printed_only = json!({ "subtype": "success", "local_command": "context", "result": "" });
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
