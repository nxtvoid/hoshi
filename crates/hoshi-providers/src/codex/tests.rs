use super::*;
use hoshi_core::{BackgroundTask, Effort, SubagentEffort};

fn request() -> TurnRequest {
    TurnRequest {
        cwd: "C:\\work".into(),
        prompt: "hi".into(),
        attachments: Vec::new(),
        earlier_media: Vec::new(),
        resume_session_id: None,
        resume_session_at: None,
        fork_session: false,
        handoff: None,
        model: None,
        effort: Some(Effort::High),
        fast_mode: false,
        ultracode: false,
        access_mode: AccessMode::AcceptEdits,
        plan_mode: false,
        mcp: Some(McpServer {
            url: "http://127.0.0.1:4100/mcp".into(),
            authorization: "Bearer thread-1".into(),
        }),
        instructions: None,
        subagent_effort: SubagentEffort::default(),
        environment: vec![("HOSHI_THREAD_ID".into(), "thread-1".into())],
    }
}

#[test]
fn a_thread_reaches_hoshis_mcp_server_and_its_environment() {
    let config = thread_config(&request());

    assert_eq!(
        config["mcp_servers.hoshi"],
        json!({
            "url": "http://127.0.0.1:4100/mcp",
            "http_headers": { "Authorization": "Bearer thread-1" },
            "default_tools_approval_mode": "approve",
            "tool_timeout_sec": MCP_TOOL_TIMEOUT_SECS,
        })
    );
    assert_eq!(
        config["shell_environment_policy.set.HOSHI_THREAD_ID"],
        "thread-1"
    );
}

#[test]
fn access_modes_become_codexs_approval_policy_and_sandbox() {
    assert_eq!(policy(AccessMode::Ask).0, "untrusted");
    assert_eq!(policy(AccessMode::Ask).2["type"], "readOnly");
    assert_eq!(policy(AccessMode::AcceptEdits).0, "on-request");
    assert_eq!(policy(AccessMode::AcceptEdits).2["type"], "workspaceWrite");
    assert_eq!(policy(AccessMode::Auto).1, "auto_review");
    assert_eq!(policy(AccessMode::FullAccess).0, "never");
    assert_eq!(policy(AccessMode::FullAccess).2["type"], "dangerFullAccess");
}

#[test]
fn a_turn_carries_its_model_effort_and_mode() {
    let settings = TurnSettings {
        model: "gpt-6-luna".into(),
        effort: Some("high".into()),
        fast_tier: Some("priority".into()),
        access: AccessMode::FullAccess,
        cwd: "C:\\work".into(),
    };

    let params = settings.turn_params("t1", &[text_input("hi")], true);

    assert_eq!(params["threadId"], "t1");
    assert_eq!(params["model"], "gpt-6-luna");
    assert_eq!(params["effort"], "high");
    assert_eq!(params["serviceTier"], "priority");
    assert_eq!(params["approvalPolicy"], "never");
    assert_eq!(
        params["collaborationMode"],
        json!({
            "mode": "plan",
            "settings": { "model": "gpt-6-luna", "reasoning_effort": "high", "developer_instructions": null },
        })
    );
    assert_eq!(
        params["input"],
        json!([{ "type": "text", "text": "hi", "text_elements": [] }])
    );
}

#[test]
fn images_go_by_path_and_files_by_a_line_of_the_text() {
    let dir = std::env::temp_dir().join(format!("hoshi-codex-input-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let image = dir.join("shot.png");
    std::fs::write(&image, b"png").unwrap();
    let attachments = [
        TurnAttachment {
            attachment: MessageAttachment::Image {
                id: "i1".into(),
                name: "shot.png".into(),
                mime_type: "image/png".into(),
                size: 3,
            },
            path: Some(image.clone()),
            extracted_text: None,
        },
        TurnAttachment {
            attachment: MessageAttachment::File {
                id: "f1".into(),
                name: "notes.txt".into(),
                mime_type: "text/plain".into(),
                size: 3,
            },
            path: Some(dir.join("notes.txt")),
            extracted_text: None,
        },
        TurnAttachment {
            attachment: MessageAttachment::Skill {
                name: "review".into(),
            },
            path: None,
            extracted_text: None,
        },
    ];
    let skills = HashMap::from([(
        "review".to_owned(),
        "C:\\skills\\review\\SKILL.md".to_owned(),
    )]);

    let input = user_input("look at $review", &attachments, &skills);

    let text = input[0]["text"].as_str().unwrap();
    assert!(text.starts_with("look at $review\n\n[Image #1 \"shot.png\" is saved at: "));
    assert!(text.contains("[File #1 \"notes.txt\" is saved at: "));
    assert_eq!(input[1], json!({ "type": "localImage", "path": image }));
    assert_eq!(
        input[2],
        json!({ "type": "skill", "name": "review", "path": "C:\\skills\\review\\SKILL.md" })
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn codex_finds_hoshis_tools_without_a_search() {
    let instructions = format!("Use mcp__hoshi__delegate_task{}.", crate::DEFERRED_TOOLS);
    assert_eq!(
        codex_instructions(&instructions),
        "Use mcp__hoshi__delegate_task."
    );
}

#[test]
fn one_shot_answers_are_read_from_their_json() {
    assert_eq!(
        parse_title("{\"title\":\"Mobile Safari Login Issue\",\"needsRefinement\":false}\n"),
        Some(GeneratedTitle {
            title: "Mobile Safari Login Issue".into(),
            needs_refinement: false,
        })
    );
    assert_eq!(parse_title("error: not signed in"), None);
    let commit = structured("```json\n{\"subject\":\"fix: keep the login\",\"body\":\"\"}\n```")
        .and_then(|output| writing::parse_commit(&output));
    assert_eq!(
        commit.map(|commit| commit.subject),
        Some("fix: keep the login".into())
    );
}

fn rpc_error(code: i64, message: &str) -> RpcError {
    RpcError {
        code,
        message: message.into(),
        data: Value::Null,
    }
}

type Opened = Result<(String, Option<String>), TurnFailure>;

/// Opens the thread `t1` resumes against answers given in turn, and the calls made.
fn resume_with(answers: Vec<Result<Value, RpcError>>) -> (Opened, Vec<String>) {
    let answers = Mutex::new(answers.into_iter());
    let calls = Mutex::new(Vec::new());
    let call = |method: &str, _params: Value| {
        lock(&calls).push(method.to_owned());
        lock(&answers)
            .next()
            .unwrap_or_else(|| Err(RpcError::gone()))
    };
    let request = TurnRequest {
        resume_session_id: Some("t1".into()),
        ..request()
    };
    let opened = open_thread(&call, &request, Duration::ZERO);
    (opened, calls.into_inner().unwrap())
}

#[test]
fn a_fork_opens_a_copy_of_the_thread_through_its_turn_unarchiving_it_first() {
    let answers = Mutex::new(
        vec![
            Err(rpc_error(-32600, "session t1 is archived")),
            Ok(json!({})),
            Ok(json!({ "thread": { "id": "t2" } })),
        ]
        .into_iter(),
    );
    let calls = Mutex::new(Vec::new());
    let call = |method: &str, params: Value| {
        lock(&calls).push((method.to_owned(), params));
        lock(&answers)
            .next()
            .unwrap_or_else(|| Err(RpcError::gone()))
    };
    let request = TurnRequest {
        resume_session_id: Some("t1".into()),
        resume_session_at: Some("turn-1".into()),
        fork_session: true,
        ..request()
    };

    let opened = open_thread(&call, &request, Duration::ZERO);

    assert_eq!(opened.unwrap().0, "t2");
    let calls = calls.into_inner().unwrap();
    let methods: Vec<&str> = calls.iter().map(|(method, _)| method.as_str()).collect();
    assert_eq!(methods, ["thread/fork", "thread/unarchive", "thread/fork"]);
    let fork = &calls[2].1;
    assert_eq!(fork["threadId"], "t1");
    assert_eq!(fork["lastTurnId"], "turn-1");
    assert_eq!(fork["excludeTurns"], true);

    let whole = Mutex::new(Vec::new());
    let call = |method: &str, params: Value| {
        lock(&whole).push((method.to_owned(), params));
        Ok(json!({ "thread": { "id": "t3" } }))
    };
    let request = TurnRequest {
        resume_session_at: None,
        ..request
    };
    assert_eq!(
        open_thread(&call, &request, Duration::ZERO).unwrap().0,
        "t3"
    );
    let whole = whole.into_inner().unwrap();
    assert_eq!(whole[0].0, "thread/fork");
    assert!(whole[0].1.get("lastTurnId").is_none());
}

#[test]
fn a_handoff_goes_in_as_the_conversation_s_own_items() {
    let handoff = ContextHandoff {
        context: "Context handoff (full_thread_summary):".into(),
        messages: vec![hoshi_core::HistoricalMessage {
            role: hoshi_core::HistoricalRole::Assistant,
            kind: "assistant_message",
            text: "Done.".into(),
            thread_id: "t".into(),
            turn_id: Some("m1".into()),
            item_id: "a1".into(),
            status: "completed",
            turn_status: Some(hoshi_core::TurnStatus::Completed),
        }],
        omitted_items: 0,
    };

    let items = history_items(&handoff);

    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["role"], "user");
    assert_eq!(items[0]["content"][0]["type"], "input_text");
    assert_eq!(items[1]["role"], "assistant");
    assert_eq!(items[1]["content"][0]["type"], "output_text");
    assert!(
        items[1]["content"][0]["text"]
            .as_str()
            .unwrap()
            .ends_with("item=a1; status=completed; turn-status=completed]\nDone.")
    );
}

#[test]
fn a_thread_that_cant_be_resumed_fails_the_turn_instead_of_starting_over() {
    let (opened, calls) = resume_with(vec![Err(rpc_error(
        -32600,
        "no rollout found for thread id t1",
    ))]);
    assert_eq!(
        opened,
        Err(TurnFailure::ResumeFailed {
            detail: "no rollout found for thread id t1".into()
        })
    );
    assert_eq!(calls, ["thread/resume"]);

    let (opened, calls) = resume_with(vec![Err(rpc_error(0, "no answer to thread/resume"))]);
    assert!(matches!(opened, Err(TurnFailure::ResumeFailed { .. })));
    assert_eq!(calls, ["thread/resume"]);
}

#[test]
fn an_archived_thread_is_unarchived_and_a_closing_one_waited_for() {
    let (opened, calls) = resume_with(vec![
        Err(rpc_error(
            -32600,
            "session t1 is archived; run `codex unarchive t1`",
        )),
        Ok(json!({})),
        Err(rpc_error(-32600, "thread t1 is closing")),
        Ok(json!({ "thread": { "id": "t1" }, "model": "gpt-6-luna" })),
    ]);

    assert_eq!(opened, Ok(("t1".into(), Some("gpt-6-luna".into()))));
    assert_eq!(
        calls,
        [
            "thread/resume",
            "thread/unarchive",
            "thread/resume",
            "thread/resume"
        ]
    );
}

#[test]
fn a_thread_another_client_writes_is_in_use() {
    let writer = || Err(rpc_error(-32600, "thread t1 already has an active writer"));
    let (opened, calls) = resume_with((0..=CLOSING_TRIES).map(|_| writer()).collect());
    assert_eq!(calls.len(), CLOSING_TRIES + 1);
    assert_eq!(
        opened,
        Err(TurnFailure::ResumeFailed {
            detail: "thread in use: thread t1 already has an active writer".into()
        })
    );
}

#[test]
fn turn_errors_become_the_failures_hoshi_knows() {
    let error = |info: Value| json!({ "message": "turn failed", "codexErrorInfo": info, "additionalDetails": "  upstream said no  " });
    let limits = UsageLimits {
            windows: catalog::usage_update(&json!({ "rateLimits": {
                "limitId": "codex",
                "primary": { "usedPercent": 100, "windowDurationMins": 300, "resetsAt": 1_795_561_547 },
                "secondary": { "usedPercent": 40, "windowDurationMins": 10_080, "resetsAt": 1_796_000_000 },
            } }))
            .unwrap()
            .windows,
            banked_resets: None,
            account: None,
            extra_usage: None,
        };
    let detail = || "upstream said no".to_owned();

    assert_eq!(
        turn_failure(&error(json!("usageLimitExceeded")), Some(&limits)),
        TurnFailure::UsageLimited {
            detail: detail(),
            resets_at: Some("2026-11-24T23:05:47Z".into()),
        }
    );
    assert_eq!(
        turn_failure(&error(json!("usageLimitExceeded")), None),
        TurnFailure::UsageLimited {
            detail: detail(),
            resets_at: None,
        }
    );
    assert_eq!(
        turn_failure(&error(json!("rateLimitExceeded")), None),
        TurnFailure::Overloaded { detail: detail() }
    );
    assert_eq!(
        turn_failure(
            &error(json!({ "responseTooManyFailedAttempts": { "httpStatusCode": 429 } })),
            None
        ),
        TurnFailure::Overloaded { detail: detail() }
    );
    assert_eq!(
        turn_failure(&error(json!("unauthorized")), None),
        TurnFailure::AuthenticationFailed { detail: detail() }
    );
    assert_eq!(
        turn_failure(&error(json!("contextWindowExceeded")), None),
        TurnFailure::ContextFull { detail: detail() }
    );
    assert_eq!(
        turn_failure(
            &json!({ "message": "boom", "codexErrorInfo": "other" }),
            None
        ),
        TurnFailure::ProviderError {
            detail: "boom".into()
        }
    );
    assert_eq!(
        latest_error(
            &json!({ "message": "turn failed", "codexErrorInfo": null }),
            Some(json!({ "message": "earlier", "codexErrorInfo": "contextWindowExceeded" })),
        ),
        json!({ "message": "turn failed", "codexErrorInfo": "contextWindowExceeded" })
    );
}

#[test]
fn retry_messages_count_their_attempts() {
    assert_eq!(retry_progress("Reconnecting... 2/5"), Some((2, 5)));
    assert_eq!(retry_progress("Reconnecting... (3 / 10)"), Some((3, 10)));
    assert_eq!(retry_progress("Reconnecting..."), None);
    assert_eq!(retry_progress("see https://status.openai.com/v2"), None);
    assert_eq!(retry_progress("0/5"), None);
}

fn recorded() -> (OutputSink, Arc<Mutex<Vec<ProviderOutput>>>) {
    let outputs = Arc::new(Mutex::new(Vec::new()));
    let sink = outputs.clone();
    (Box::new(move |output| lock(&sink).push(output)), outputs)
}

#[test]
fn a_retry_shows_and_one_that_stalls_fails_the_turn() {
    let (sink, outputs) = recorded();
    let turn = Turn::new(sink, Arc::default());
    turn.opened("t".into());
    let retry = |message: &str| {
        json!({ "threadId": "t", "turnId": "u", "willRetry": true, "error": {
            "message": message,
            "codexErrorInfo": { "responseStreamDisconnected": { "httpStatusCode": 429 } },
            "additionalDetails": "stream disconnected before completion",
        } })
    };

    turn.notification("error", &retry("Reconnecting... 1/5"));
    turn.notification("error", &retry("Reconnecting..."));
    let now = Instant::now();
    let waiting = turn.stalled_retry(now + RETRY_STALL_TIMEOUT / 2);
    let stalled = turn.stalled_retry(now + RETRY_STALL_TIMEOUT);
    turn.notification(
        "item/agentMessage/delta",
        &json!({ "threadId": "t", "turnId": "u", "itemId": "m", "delta": "hi" }),
    );
    let answered = turn.stalled_retry(now + RETRY_STALL_TIMEOUT);

    let retrying = |attempt, max_attempts| ProviderOutput::Retrying {
        attempt,
        max_attempts,
        retry_in_ms: None,
        error: Some("stream disconnected before completion".into()),
    };
    assert_eq!(
        lock(&outputs)[..2],
        [retrying(1, Some(5)), retrying(2, None)]
    );
    assert_eq!(waiting, None);
    assert_eq!(
        stalled,
        Some(TurnFailure::Overloaded {
            detail: "stream disconnected before completion".into()
        })
    );
    assert_eq!(answered, None);
}

#[test]
fn approvals_for_the_session_end_with_the_access_mode_they_were_given_in() {
    let mut approvals = SessionApprovals::default();
    approvals.begin("t", AccessMode::AcceptEdits);
    approvals.grant("t", AccessMode::AcceptEdits, Kind::Command);

    let kept = approvals.allows("t", AccessMode::AcceptEdits, Kind::Command);
    let elsewhere = approvals.allows("t", AccessMode::Ask, Kind::Command);
    approvals.begin("t", AccessMode::Ask);
    approvals.begin("t", AccessMode::AcceptEdits);

    assert!(kept);
    assert!(!elsewhere);
    assert!(!approvals.allows("t", AccessMode::AcceptEdits, Kind::Command));
    assert!(!approvals.allows("t", AccessMode::AcceptEdits, Kind::FileChange));
}

#[test]
fn a_turn_that_leaves_commands_running_keeps_its_server_until_they_end() {
    let (sink, outputs) = recorded();
    let turn = Turn::new(sink, Arc::default());
    turn.opened("t".into());
    let command = json!({ "type": "commandExecution", "id": "exec-1", "command": "npm run dev", "processId": "pty-1", "status": "inProgress" });
    turn.notification(
        "item/started",
        &json!({ "threadId": "t", "turnId": "u", "startedAtMs": 1_000, "item": command }),
    );

    turn.end(TurnOutcome::Completed);
    let lingering = turn.lingering.load(Ordering::SeqCst);
    let mut done = command.clone();
    done["status"] = json!("completed");
    turn.notification(
        "item/completed",
        &json!({ "threadId": "t", "turnId": "u", "item": done }),
    );
    turn.gone(String::new());

    let task = BackgroundTask {
        task_id: "exec-1".into(),
        tool_call_id: Some("exec-1".into()),
        description: "npm run dev".into(),
        started_at: 1_000,
    };
    assert!(lingering);
    assert_eq!(
        lock(&outputs)[1..],
        [
            ProviderOutput::ToolCallCompleted {
                item_id: "exec-1".into(),
                output: String::new(),
                is_error: false,
                detail: None,
            },
            ProviderOutput::BackgroundTasks {
                tasks: vec![task],
                waiting: false,
            },
            ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Completed
            },
            ProviderOutput::BackgroundTasks {
                tasks: Vec::new(),
                waiting: false,
            },
            ProviderOutput::ProcessEnded,
        ]
    );
}

#[test]
fn a_turn_that_ends_otherwise_keeps_nothing_running() {
    let (sink, outputs) = recorded();
    let turn = Turn::new(sink, Arc::default());
    turn.opened("t".into());
    turn.notification(
            "item/started",
            &json!({ "threadId": "t", "item": { "type": "commandExecution", "id": "exec-1", "command": "sleep 9", "status": "inProgress" } }),
        );

    turn.end(TurnOutcome::Interrupted);
    turn.gone(String::new());

    assert!(!turn.lingering.load(Ordering::SeqCst));
    assert_eq!(
        lock(&outputs)[1..],
        [ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Interrupted
        }]
    );
}
