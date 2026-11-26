use super::*;

const THREAD: &str = "01a1db17-c7a7-78a2-bd20-0b0c056d8800";
const TURN: &str = "01a1db17-c7e2-7752-823f-e0b4b42b3b80";

fn feed(conversation: &mut Conversation, lines: &[&str]) -> Vec<ProviderOutput> {
    lines
        .iter()
        .flat_map(|line| {
            let message: Value = serde_json::from_str(line).unwrap();
            conversation.notification(message["method"].as_str().unwrap(), &message["params"])
        })
        .collect()
}

#[test]
fn a_message_streams_and_ends_with_its_whole_text() {
    let mut conversation = Conversation::new(THREAD.into());
    let outputs = feed(
        &mut conversation,
        &[
            r#"{"method":"turn/started","params":{"threadId":"01a1db17-c7a7-78a2-bd20-0b0c056d8800","turn":{"id":"01a1db17-c7e2-7752-823f-e0b4b42b3b80","items":[],"status":"inProgress"}}}"#,
            r#"{"method":"item/started","params":{"item":{"type":"agentMessage","id":"msg_1","text":"","phase":"final_answer"},"threadId":"01a1db17-c7a7-78a2-bd20-0b0c056d8800","turnId":"01a1db17-c7e2-7752-823f-e0b4b42b3b80"}}"#,
            r#"{"method":"item/agentMessage/delta","params":{"threadId":"01a1db17-c7a7-78a2-bd20-0b0c056d8800","turnId":"01a1db17-c7e2-7752-823f-e0b4b42b3b80","itemId":"msg_1","delta":"fixture"}}"#,
            r#"{"method":"item/agentMessage/delta","params":{"threadId":"01a1db17-c7a7-78a2-bd20-0b0c056d8800","turnId":"01a1db17-c7e2-7752-823f-e0b4b42b3b80","itemId":"msg_1","delta":" ok"}}"#,
            r#"{"method":"item/completed","params":{"item":{"type":"agentMessage","id":"msg_1","text":"fixture ok","phase":"final_answer"},"threadId":"01a1db17-c7a7-78a2-bd20-0b0c056d8800","turnId":"01a1db17-c7e2-7752-823f-e0b4b42b3b80"}}"#,
            r#"{"method":"thread/tokenUsage/updated","params":{"threadId":"01a1db17-c7a7-78a2-bd20-0b0c056d8800","turnId":"01a1db17-c7e2-7752-823f-e0b4b42b3b80","tokenUsage":{"total":{"totalTokens":27456,"inputTokens":27311,"cachedInputTokens":13440,"outputTokens":145,"reasoningOutputTokens":0},"last":{"totalTokens":13767,"inputTokens":13719,"cachedInputTokens":13440,"outputTokens":48,"reasoningOutputTokens":0},"modelContextWindow":258400}}}"#,
        ],
    );

    assert_eq!(conversation.turn_id(), Some(TURN));
    assert_eq!(
        outputs,
        [
            ProviderOutput::TextDelta {
                item_id: "msg_1".into(),
                delta: "fixture".into()
            },
            ProviderOutput::TextDelta {
                item_id: "msg_1".into(),
                delta: " ok".into()
            },
            ProviderOutput::AssistantMessage {
                item_id: "msg_1".into(),
                text: "fixture ok".into()
            },
            ProviderOutput::ContextUsage(ContextUsage {
                used_tokens: 13_767,
                max_tokens: Some(258_400),
                cache_ttl_secs: None,
                cache_expires_at: None,
            }),
        ]
    );
}

#[test]
fn reasoning_streams_its_summary_parts() {
    let mut translator = Translator::default();
    let delta =
        |index: u64, text: &str| json!({ "itemId": "rs_1", "summaryIndex": index, "delta": text });
    let mut outputs =
        translator.notification("item/reasoning/summaryTextDelta", &delta(0, "**Planning**"));
    outputs.extend(translator.notification(
        "item/reasoning/summaryPartAdded",
        &json!({ "itemId": "rs_1", "summaryIndex": 0 }),
    ));
    outputs.extend(translator.notification(
        "item/reasoning/summaryPartAdded",
        &json!({ "itemId": "rs_1", "summaryIndex": 1 }),
    ));
    outputs.extend(translator.notification(
            "item/completed",
            &json!({ "item": { "type": "reasoning", "id": "rs_1", "summary": ["**Planning**", "Then"], "content": [] } }),
        ));
    outputs.extend(translator.notification(
        "item/completed",
        &json!({ "item": { "type": "reasoning", "id": "rs_2", "summary": [], "content": [] } }),
    ));

    assert_eq!(
        outputs,
        [
            ProviderOutput::ReasoningDelta {
                item_id: "rs_1".into(),
                delta: "**Planning**".into()
            },
            ProviderOutput::ReasoningDelta {
                item_id: "rs_1".into(),
                delta: "\n\n".into()
            },
            ProviderOutput::Reasoning {
                item_id: "rs_1".into(),
                text: "**Planning**\n\nThen".into()
            },
        ]
    );
}

#[test]
fn a_command_that_reads_a_file_shows_as_read() {
    let started: Value = serde_json::from_str(r#"{"type":"commandExecution","id":"exec-61116b80","command":"\"C:\\\\WINDOWS\\\\System32\\\\WindowsPowerShell\\\\v1.0\\\\powershell.exe\" -Command 'type note.txt'","cwd":"C:\\work","processId":null,"source":"agent","status":"inProgress","commandActions":[{"type":"read","command":"type note.txt","name":"note.txt","path":"C:\\work\\note.txt"}],"aggregatedOutput":null,"exitCode":null}"#).unwrap();
    let mut completed = started.clone();
    completed["status"] = json!("completed");
    completed["aggregatedOutput"] = json!("The secret word is pelican.\r\n");
    completed["exitCode"] = json!(0);
    let mut translator = Translator::default();

    let mut outputs = translator.notification("item/started", &json!({ "item": started }));
    outputs.extend(translator.notification("item/completed", &json!({ "item": completed })));

    assert_eq!(
        outputs,
        [
            ProviderOutput::ToolCallStarted {
                item_id: "exec-61116b80".into(),
                name: "Read".into(),
                input: r#"{"file_path":"C:\\work\\note.txt"}"#.into(),
            },
            ProviderOutput::ToolCallCompleted {
                item_id: "exec-61116b80".into(),
                output: "The secret word is pelican.\r\n".into(),
                is_error: false,
                detail: None,
            },
        ]
    );
}

#[test]
fn a_command_streams_its_output_and_fails_with_its_exit_code() {
    let mut translator = Translator::default();
    let item = json!({
        "type": "commandExecution",
        "id": "exec-1",
        "command": "/bin/zsh -lc \"for i in 1 2; do echo $i; done\"",
        "commandActions": [
            { "type": "unknown", "command": "for i in 1" },
            { "type": "unknown", "command": "done" },
        ],
        "status": "inProgress",
    });
    let started = translator.notification("item/started", &json!({ "item": item }));
    translator.notification(
        "item/commandExecution/outputDelta",
        &json!({ "itemId": "exec-1", "delta": "1\n" }),
    );
    let streamed = translator.notification(
        "item/commandExecution/outputDelta",
        &json!({ "itemId": "exec-1", "delta": "2\n" }),
    );
    let mut failed = item.clone();
    failed["status"] = json!("failed");
    failed["exitCode"] = json!(1);
    failed["aggregatedOutput"] = Value::Null;
    let done = translator.notification("item/completed", &json!({ "item": failed }));

    assert_eq!(
        started,
        [ProviderOutput::ToolCallStarted {
            item_id: "exec-1".into(),
            name: "Bash".into(),
            input: r#"{"command":"for i in 1 2; do echo $i; done"}"#.into(),
        }]
    );
    assert_eq!(
        streamed,
        [ProviderOutput::ToolOutput {
            item_id: "exec-1".into(),
            output: "1\n2\n".into()
        }]
    );
    assert_eq!(
        done,
        [ProviderOutput::ToolCallCompleted {
            item_id: "exec-1".into(),
            output: "1\n2\n".into(),
            is_error: true,
            detail: None,
        }]
    );
}

#[test]
fn shell_wrappers_come_off_the_command() {
    assert_eq!(
        unwrap_shell(
            r#""C:\\WINDOWS\\System32\\WindowsPowerShell\\v1.0\\powershell.exe" -Command 'type note.txt'"#
        ),
        "type note.txt"
    );
    assert_eq!(
        unwrap_shell(r#"/bin/bash -lc "sed -n '1,240p' package.json""#),
        "sed -n '1,240p' package.json"
    );
    assert_eq!(
        unwrap_shell("pwsh -NoProfile -Command 'Write-Output ''hi'''"),
        "Write-Output 'hi'"
    );
    assert_eq!(unwrap_shell("cmd.exe /c dir"), "dir");
    assert_eq!(unwrap_shell("cargo test"), "cargo test");
    assert_eq!(unwrap_shell("bash script.sh"), "bash script.sh");
}

#[test]
fn a_new_file_writes_and_a_changed_one_edits_with_its_hunks() {
    let item: Value = serde_json::from_str(r#"{"type":"fileChange","id":"exec-8d299eb0","changes":[{"path":"C:\\work\\hello.txt","kind":{"type":"add"},"diff":"hello\nbye\n"},{"path":"C:\\work\\a.rs","kind":{"type":"update","move_path":null},"diff":"@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n"}],"status":"completed"}"#).unwrap();
    let mut translator = Translator::default();

    let sorted = |input: String| {
        let fields: std::collections::BTreeMap<String, Value> =
            serde_json::from_str(&input).unwrap();
        serde_json::to_string(&fields).unwrap()
    };
    let outputs: Vec<_> = translator
        .notification("item/completed", &json!({ "item": item }))
        .into_iter()
        .map(|output| match output {
            ProviderOutput::ToolCallStarted {
                item_id,
                name,
                input,
            } => ProviderOutput::ToolCallStarted {
                item_id,
                name,
                input: sorted(input),
            },
            other => other,
        })
        .collect();

    assert_eq!(
        outputs,
        [
            ProviderOutput::ToolCallStarted {
                item_id: "exec-8d299eb0".into(),
                name: "Write".into(),
                input: r#"{"content":"hello\nbye\n","file_path":"C:\\work\\hello.txt"}"#.into(),
            },
            ProviderOutput::ToolCallStarted {
                item_id: "exec-8d299eb0#1".into(),
                name: "Edit".into(),
                input: r#"{"file_path":"C:\\work\\a.rs","new_string":"TWO","old_string":"two"}"#
                    .into(),
            },
            ProviderOutput::ToolCallCompleted {
                item_id: "exec-8d299eb0".into(),
                output: String::new(),
                is_error: false,
                detail: Some(ToolDetail::FileChange {
                    path: "C:\\work\\hello.txt".into(),
                    additions: 2,
                    deletions: 0,
                    diff: Some("@@ -0,0 +1,2 @@\n+hello\n+bye\n".into()),
                }),
            },
            ProviderOutput::ToolCallCompleted {
                item_id: "exec-8d299eb0#1".into(),
                output: String::new(),
                is_error: false,
                detail: Some(ToolDetail::FileChange {
                    path: "C:\\work\\a.rs".into(),
                    additions: 1,
                    deletions: 1,
                    diff: Some("@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n".into()),
                }),
            },
        ]
    );
}

#[test]
fn hoshis_mcp_tools_keep_their_names() {
    let started: Value = serde_json::from_str(r#"{"type":"mcpToolCall","id":"exec-632dd7bd","server":"hoshi","tool":"echo_text","status":"inProgress","arguments":{"text":"kiwi"},"result":null,"error":null}"#).unwrap();
    let completed: Value = serde_json::from_str(r#"{"type":"mcpToolCall","id":"exec-632dd7bd","server":"hoshi","tool":"echo_text","status":"completed","arguments":{"text":"kiwi"},"result":{"content":[{"type":"text","text":"KIWI"}],"structuredContent":null,"_meta":null},"error":null}"#).unwrap();
    let mut translator = Translator::default();

    let mut outputs = translator.notification("item/started", &json!({ "item": started }));
    outputs.extend(translator.notification("item/completed", &json!({ "item": completed })));

    assert_eq!(
        outputs,
        [
            ProviderOutput::ToolCallStarted {
                item_id: "exec-632dd7bd".into(),
                name: "mcp__hoshi__echo_text".into(),
                input: r#"{"text":"kiwi"}"#.into(),
            },
            ProviderOutput::ToolCallCompleted {
                item_id: "exec-632dd7bd".into(),
                output: "KIWI".into(),
                is_error: false,
                detail: None,
            },
        ]
    );
}

#[test]
fn a_search_shows_once_its_query_is_known() {
    let mut translator = Translator::default();
    let started = translator.notification(
            "item/started",
            &json!({ "item": { "type": "webSearch", "id": "ws_1", "query": "", "action": null, "results": null } }),
        );
    let completed = translator.notification(
            "item/completed",
            &json!({ "item": {
                "type": "webSearch",
                "id": "ws_1",
                "query": "world cup tickets",
                "action": { "type": "search", "query": "world cup tickets", "queries": null },
                "results": [{ "type": "text_result", "title": "Tickets", "url": "https://example.com", "snippet": "Prices" }],
            } }),
        );

    assert_eq!(started, []);
    assert_eq!(
        completed,
        [
            ProviderOutput::ToolCallStarted {
                item_id: "ws_1".into(),
                name: "WebSearch".into(),
                input: r#"{"query":"world cup tickets"}"#.into(),
            },
            ProviderOutput::ToolCallCompleted {
                item_id: "ws_1".into(),
                output: "Tickets\nhttps://example.com\nPrices".into(),
                is_error: false,
                detail: None,
            },
        ]
    );
}

#[test]
fn the_plan_becomes_the_task_list_and_a_proposed_plan_is_kept() {
    let mut translator = Translator::default();
    let tasks = translator.notification(
            "turn/plan/updated",
            &serde_json::from_str(r#"{"threadId":"t","turnId":"u","explanation":null,"plan":[{"step":"Read note.txt","status":"completed"},{"step":"Report the secret word","status":"inProgress"}]}"#).unwrap(),
        );
    translator.notification(
        "item/completed",
        &json!({ "item": { "type": "plan", "id": "u-plan", "text": "1. **Find the boundary.**" } }),
    );

    assert_eq!(
        tasks,
        [ProviderOutput::TaskList {
            tasks: vec![
                Task {
                    subject: "Read note.txt".into(),
                    active_form: None,
                    status: TaskStatus::Completed,
                },
                Task {
                    subject: "Report the secret word".into(),
                    active_form: None,
                    status: TaskStatus::InProgress,
                },
            ]
        }]
    );
    assert_eq!(
        translator.take_plan().as_deref(),
        Some("1. **Find the boundary.**")
    );
    assert_eq!(translator.take_plan(), None);
}

#[test]
fn compaction_reports_the_context_before_and_after() {
    let mut translator = Translator::default();
    let usage = |used: u64| json!({ "tokenUsage": { "last": { "totalTokens": used }, "modelContextWindow": 258_400 } });
    translator.notification("thread/tokenUsage/updated", &usage(13_403));
    let started = translator.notification(
        "item/started",
        &json!({ "item": { "type": "contextCompaction", "id": "c1" } }),
    );
    translator.notification("thread/tokenUsage/updated", &usage(4_000));
    let finished = translator.notification(
        "item/completed",
        &json!({ "item": { "type": "contextCompaction", "id": "c1" } }),
    );

    assert_eq!(started, [ProviderOutput::CompactionStarted]);
    assert_eq!(
        finished,
        [ProviderOutput::CompactionFinished {
            before_tokens: 13_403,
            after_tokens: 4_000
        }]
    );
}

const PARENT: &str = "01a1db37-7afd-7f60-95c4-9ccb838270f8";
const CHILD: &str = "01a1db37-87e4-74c2-83fd-ce04f18b0473";

#[test]
fn a_subagent_shows_as_a_task_and_ends_with_its_answer() {
    let mut conversation = Conversation::new(PARENT.into());
    let outputs = feed(
        &mut conversation,
        &[
            r#"{"method":"item/started","params":{"item":{"type":"subAgentActivity","model":"gpt-6-luna","reasoningEffort":"low","id":"call_08d8e71130444d6d86175e6977898607","kind":"started","agentThreadId":"01a1db37-87e4-74c2-83fd-ce04f18b0473","agentPath":"/root/hello"},"threadId":"01a1db37-7afd-7f60-95c4-9ccb838270f8","turnId":"01a1db37-7b47-7003-a3c4-421dfcb855b5"}}"#,
            r#"{"method":"item/completed","params":{"item":{"type":"subAgentActivity","model":"gpt-6-luna","reasoningEffort":"low","id":"call_08d8e71130444d6d86175e6977898607","kind":"started","agentThreadId":"01a1db37-87e4-74c2-83fd-ce04f18b0473","agentPath":"/root/hello"},"threadId":"01a1db37-7afd-7f60-95c4-9ccb838270f8","turnId":"01a1db37-7b47-7003-a3c4-421dfcb855b5"}}"#,
            r#"{"method":"turn/started","params":{"threadId":"01a1db37-87e4-74c2-83fd-ce04f18b0473","turn":{"id":"01a1db37-8812-7fc2-beb5-215b4f4d04e1","status":"inProgress"}}}"#,
            r#"{"method":"item/started","params":{"item":{"type":"collabAgentToolCall","id":"call_f4d68e2f556441b28b07c853bab92372","tool":"wait","status":"inProgress","senderThreadId":"01a1db37-7afd-7f60-95c4-9ccb838270f8","receiverThreadIds":[],"prompt":null,"model":null,"reasoningEffort":null,"agentsStates":{}},"threadId":"01a1db37-7afd-7f60-95c4-9ccb838270f8","turnId":"01a1db37-7b47-7003-a3c4-421dfcb855b5"}}"#,
            r#"{"method":"item/started","params":{"item":{"type":"agentMessage","id":"msg_04ef911e2bd01b7a016af8a1ae38cc87d29a57cfef3b58dda4","text":"","phase":"final_answer","questions":null},"threadId":"01a1db37-87e4-74c2-83fd-ce04f18b0473","turnId":"01a1db37-8812-7fc2-beb5-215b4f4d04e1"}}"#,
            r#"{"method":"item/completed","params":{"item":{"type":"agentMessage","id":"msg_04ef911e2bd01b7a016af8a1ae38cc87d29a57cfef3b58dda4","text":"Hello.","phase":"final_answer","questions":null},"threadId":"01a1db37-87e4-74c2-83fd-ce04f18b0473","turnId":"01a1db37-8812-7fc2-beb5-215b4f4d04e1"}}"#,
            r#"{"method":"item/started","params":{"item":{"type":"subAgentActivity","model":null,"reasoningEffort":null,"id":"subagent-completed-01a1db37-8812-7fc2-beb5-215b4f4d04e1","kind":"completed","agentThreadId":"01a1db37-87e4-74c2-83fd-ce04f18b0473","agentPath":"/root/hello"},"threadId":"01a1db37-7afd-7f60-95c4-9ccb838270f8","turnId":"01a1db37-7b47-7003-a3c4-421dfcb855b5"}}"#,
            r#"{"method":"turn/completed","params":{"threadId":"01a1db37-87e4-74c2-83fd-ce04f18b0473","turn":{"id":"01a1db37-8812-7fc2-beb5-215b4f4d04e1","status":"completed","error":null}}}"#,
            r#"{"method":"item/completed","params":{"item":{"type":"subAgentActivity","model":null,"reasoningEffort":null,"id":"subagent-completed-01a1db37-8812-7fc2-beb5-215b4f4d04e1","kind":"completed","agentThreadId":"01a1db37-87e4-74c2-83fd-ce04f18b0473","agentPath":"/root/hello"},"threadId":"01a1db37-7afd-7f60-95c4-9ccb838270f8","turnId":"01a1db37-7b47-7003-a3c4-421dfcb855b5"}}"#,
            r#"{"method":"item/completed","params":{"item":{"type":"collabAgentToolCall","id":"call_f4d68e2f556441b28b07c853bab92372","tool":"wait","status":"completed","senderThreadId":"01a1db37-7afd-7f60-95c4-9ccb838270f8","receiverThreadIds":[],"prompt":null,"model":null,"reasoningEffort":null,"agentsStates":{}},"threadId":"01a1db37-7afd-7f60-95c4-9ccb838270f8","turnId":"01a1db37-7b47-7003-a3c4-421dfcb855b5"}}"#,
        ],
    );

    let call = "call_08d8e71130444d6d86175e6977898607";
    assert_eq!(
        outputs,
        [
            ProviderOutput::ToolCallStarted {
                item_id: call.into(),
                name: "Task".into(),
                input: r#"{"description":"hello"}"#.into(),
            },
            ProviderOutput::ToolCallDetail {
                item_id: call.into(),
                detail: ToolDetail::Subagent {
                    model: "gpt-6-luna".into(),
                    effort: Some(Effort::Low),
                    resume_id: None,
                },
            },
            ProviderOutput::Subagent {
                tool_use_id: call.into(),
                output: Box::new(ProviderOutput::AssistantMessage {
                    item_id: "msg_04ef911e2bd01b7a016af8a1ae38cc87d29a57cfef3b58dda4".into(),
                    text: "Hello.".into(),
                }),
            },
            ProviderOutput::ToolCallCompleted {
                item_id: call.into(),
                output: "Hello.".into(),
                is_error: false,
                detail: None,
            },
        ]
    );
    assert!(conversation.tool(PARENT, call).is_some());
    assert_eq!(conversation.subagent_turns(call), []);
    assert_eq!(conversation.agents[CHILD].call, call);
}

#[test]
fn a_spawned_agent_shows_its_model_and_runs_until_its_turn_ends() {
    let mut conversation = Conversation::new("p".into());
    let spawn = json!({
        "type": "collabAgentToolCall",
        "id": "exec-27515368",
        "tool": "spawnAgent",
        "status": "inProgress",
        "senderThreadId": "p",
        "receiverThreadIds": [],
        "prompt": "Read package.json in the current workspace. Report its key scripts.",
        "model": "",
        "reasoningEffort": "medium",
    });
    let mut spawned = spawn.clone();
    spawned["status"] = json!("completed");
    spawned["receiverThreadIds"] = json!(["c"]);
    spawned["model"] = json!("gpt-5.6-luna");
    let params = |thread: &str, item: Value| json!({ "threadId": thread, "item": item });

    let mut outputs = conversation.notification("item/started", &params("p", spawn));
    outputs.extend(conversation.notification("item/completed", &params("p", spawned)));
    conversation.notification(
        "turn/started",
        &json!({ "threadId": "c", "turn": { "id": "ct" } }),
    );
    let running = conversation.subagent_turns("exec-27515368");
    outputs.extend(conversation.notification(
            "item/started",
            &params(
                "c",
                json!({ "type": "commandExecution", "id": "exec-5fe6", "command": "/bin/bash -lc \"ls\"", "commandActions": [] }),
            ),
        ));
    outputs.extend(conversation.notification(
        "turn/completed",
        &json!({ "threadId": "c", "turn": { "id": "ct", "status": "completed" } }),
    ));

    assert_eq!(running, [("c".to_owned(), "ct".to_owned())]);
    assert_eq!(
            outputs,
            [
                ProviderOutput::ToolCallStarted {
                    item_id: "exec-27515368".into(),
                    name: "Task".into(),
                    input: r#"{"description":"Read package.json in the current workspace","prompt":"Read package.json in the current workspace. Report its key scripts."}"#.into(),
                },
                ProviderOutput::ToolCallDetail {
                    item_id: "exec-27515368".into(),
                    detail: ToolDetail::Subagent {
                        model: "gpt-5.6-luna".into(),
                        effort: Some(Effort::Medium),
                        resume_id: None,
                    },
                },
                ProviderOutput::Subagent {
                    tool_use_id: "exec-27515368".into(),
                    output: Box::new(ProviderOutput::ToolCallStarted {
                        item_id: "exec-5fe6".into(),
                        name: "Bash".into(),
                        input: r#"{"command":"ls"}"#.into(),
                    }),
                },
                ProviderOutput::ToolCallCompleted {
                    item_id: "exec-27515368".into(),
                    output: String::new(),
                    is_error: false,
                    detail: None,
                },
            ]
        );
}

#[test]
fn threads_the_turn_did_not_start_are_ignored() {
    let mut conversation = Conversation::new("p".into());
    let outputs = conversation.notification(
            "item/completed",
            &json!({ "threadId": "other", "item": { "type": "agentMessage", "id": "m", "text": "hi" } }),
        );
    assert_eq!(outputs, []);
}

#[test]
fn a_proposed_plan_streams_under_the_request_it_becomes() {
    let mut translator = Translator::default();
    let delta =
        |text: &str| json!({ "threadId": "t", "turnId": "u", "itemId": "u-plan", "delta": text });

    let mut outputs = translator.notification("item/plan/delta", &delta("1. **Find"));
    outputs.extend(translator.notification("item/plan/delta", &delta("")));
    outputs.extend(translator.notification("item/plan/delta", &delta(" the boundary.**")));

    assert_eq!(plan_request_id("u"), "plan-u");
    assert_eq!(
        outputs,
        [
            ProviderOutput::TextDelta {
                item_id: "plan-u".into(),
                delta: "1. **Find".into()
            },
            ProviderOutput::TextDelta {
                item_id: "plan-u".into(),
                delta: " the boundary.**".into()
            },
        ]
    );
}

#[test]
fn raw_reasoning_streams_when_its_summary_does_not() {
    let mut translator = Translator::default();
    let text = |item: &str, index: u64, delta: &str| json!({ "threadId": "t", "turnId": "u", "itemId": item, "contentIndex": index, "delta": delta });

    let mut outputs = translator.notification("item/reasoning/textDelta", &text("rs_1", 0, "Look"));
    outputs.extend(translator.notification("item/reasoning/textDelta", &text("rs_1", 0, " first")));
    outputs.extend(translator.notification("item/reasoning/textDelta", &text("rs_1", 1, "Then")));
    outputs.extend(translator.notification(
        "item/reasoning/summaryTextDelta",
        &json!({ "itemId": "rs_2", "summaryIndex": 0, "delta": "**Plan**" }),
    ));
    outputs.extend(translator.notification("item/reasoning/textDelta", &text("rs_2", 0, "raw")));

    assert_eq!(
        outputs,
        [
            ProviderOutput::ReasoningDelta {
                item_id: "rs_1".into(),
                delta: "Look".into()
            },
            ProviderOutput::ReasoningDelta {
                item_id: "rs_1".into(),
                delta: " first".into()
            },
            ProviderOutput::ReasoningDelta {
                item_id: "rs_1".into(),
                delta: "\n\nThen".into()
            },
            ProviderOutput::ReasoningDelta {
                item_id: "rs_2".into(),
                delta: "**Plan**".into()
            },
        ]
    );
}

#[test]
fn a_rerouted_turn_names_the_model_it_runs_on() {
    let mut conversation = Conversation::new("01a0d5ed-187d-70c2-b4b6-b6881a68de09".into());
    let outputs = feed(
        &mut conversation,
        &[
            r#"{"method":"model/rerouted","params":{"threadId":"01a0d5ed-187d-70c2-b4b6-b6881a68de09","turnId":"01a0d5ed-1957-7a22-8399-d0172af342fd","fromModel":"gpt-6-luna","toModel":"gpt-6-astra","reason":"highRiskCyberActivity"}}"#,
        ],
    );

    assert_eq!(
        outputs,
        [ProviderOutput::ModelRerouted {
            model: "gpt-6-astra".into(),
            reason: Some("highRiskCyberActivity".into()),
        }]
    );
}

#[test]
fn a_hook_that_stops_the_turn_says_why() {
    let mut conversation = Conversation::new("t".into());
    let hook = |turn: &str, status: &str, entries: Value| {
        json!({ "method": "hook/completed", "params": { "threadId": "t", "turnId": turn,
                "run": { "id": "h1", "eventName": "stop", "source": "project", "status": status,
                    "entries": entries } } })
        .to_string()
    };
    let stop = json!([{ "kind": "feedback", "text": "ran checks" },
            { "kind": "stop", "text": " tests fail " }]);
    let lines = [
        r#"{"method":"turn/started","params":{"threadId":"t","turn":{"id":"u1"}}}"#.to_owned(),
        hook("u1", "completed", stop.clone()),
        hook("u0", "stopped", stop.clone()),
        hook(
            "u1",
            "stopped",
            json!([{ "kind": "warning", "text": "slow" }]),
        ),
        hook("u1", "stopped", stop),
    ];
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();

    assert_eq!(
        feed(&mut conversation, &lines),
        [ProviderOutput::HookStopped {
            event: "stop".into(),
            source: Some("project".into()),
            reason: "tests fail".into(),
        }]
    );
}

#[test]
fn an_mcp_server_that_fails_to_start_is_told_once() {
    let mut conversation = Conversation::new("t".into());
    let status = |status: &str, error: Value| json!({ "threadId": "t", "name": "docs", "status": status, "error": error, "failureReason": null });

    let mut outputs = conversation.notification(
        "mcpServer/startupStatus/updated",
        &status("starting", Value::Null),
    );
    for _ in 0..2 {
        outputs.extend(conversation.notification(
            "mcpServer/startupStatus/updated",
            &status("failed", json!(" connection refused ")),
        ));
    }
    outputs.extend(conversation.notification(
        "mcpServer/startupStatus/updated",
        &status("ready", Value::Null),
    ));
    outputs.extend(conversation.notification(
        "mcpServer/startupStatus/updated",
        &status("failed", Value::Null),
    ));

    let failed = |error: &str| ProviderOutput::McpServerFailed {
        name: "docs".into(),
        error: error.into(),
    };
    assert_eq!(outputs, [failed("connection refused"), failed("")]);
}

#[test]
fn a_command_still_running_as_the_turn_completes_goes_on_in_the_background() {
    let mut conversation = Conversation::new("t".into());
    let command = json!({ "type": "commandExecution", "id": "exec-1", "command": "/bin/bash -lc \"npm run dev\"", "commandActions": [], "processId": "pty-7", "status": "inProgress" });
    let params = |extra: Value| {
        let mut params = json!({ "threadId": "t", "turnId": "u" });
        params
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        params
    };
    conversation.notification(
        "item/started",
        &params(json!({ "item": command, "startedAtMs": 1_795_000_000_000_u64 })),
    );
    conversation.notification(
        "item/commandExecution/outputDelta",
        &params(json!({ "itemId": "exec-1", "delta": "ready on :5173\n" })),
    );

    let settled = conversation.settle();
    let background = conversation.take_background_change();
    let later = conversation.notification(
        "item/commandExecution/outputDelta",
        &params(json!({ "itemId": "exec-1", "delta": "GET /\n" })),
    );
    let process = conversation.process_id("exec-1").map(str::to_owned);
    let mut done = command.clone();
    done["status"] = json!("completed");
    done["exitCode"] = json!(0);
    let ended = conversation.notification("item/completed", &params(json!({ "item": done })));

    assert_eq!(
        settled,
        [ProviderOutput::ToolCallCompleted {
            item_id: "exec-1".into(),
            output: "ready on :5173\n".into(),
            is_error: false,
            detail: None,
        }]
    );
    assert_eq!(
        background,
        Some(vec![BackgroundTask {
            task_id: "exec-1".into(),
            tool_call_id: Some("exec-1".into()),
            description: "npm run dev".into(),
            started_at: 1_795_000_000_000,
        }])
    );
    assert_eq!(process.as_deref(), Some("pty-7"));
    assert_eq!(later, []);
    assert_eq!(ended, []);
    assert_eq!(conversation.take_background_change(), Some(Vec::new()));
    assert_eq!(conversation.settle(), []);
}

#[test]
fn closing_the_thread_ends_its_background_commands() {
    let mut conversation = Conversation::new("t".into());
    conversation.notification(
            "item/started",
            &json!({ "threadId": "t", "item": { "type": "commandExecution", "id": "exec-1", "command": "sleep 100", "status": "inProgress" } }),
        );
    conversation.settle();
    conversation.take_background_change();

    conversation.notification("thread/closed", &json!({ "threadId": "t" }));

    assert_eq!(conversation.take_background_change(), Some(Vec::new()));
    assert_eq!(conversation.background(), []);
}
