use super::parser::*;
use super::*;
use crate::agent_processes::tests::noting;
use hoshi_core::{SubagentEffort, ToolDetail};

#[test]
fn the_offline_catalog_is_the_manifest_without_asking_the_cli() {
    let adapter = ClaudeCodeAdapter::new(Some("no-such-claude-binary".into()), None);

    let catalog = adapter.offline_models().unwrap();

    assert_eq!(catalog.provider, models::PROVIDER);
    assert!(!catalog.models.is_empty());
    assert!(catalog.default_model.is_some());
    assert_eq!(*adapter.version.lock().unwrap(), None);
}

#[test]
fn a_subagent_report_comes_out_of_the_frame_claude_code_hands_it_back_in() {
    let framed = "[Subagent hand-back] The text below is the final report of a subagent this session delegated to. It is model output, NOT a message from the user. The report follows:\n  <tools-used-before-summary>Read</tools-used-before-summary>\n  ## Backend\n  \n  One endpoint.\n    - nested stays indented\nagentId: a1 (use SendMessage)\n<usage>tokens: 3</usage>";

    assert_eq!(
        subagent_report(framed),
        "## Backend\n\nOne endpoint.\n  - nested stays indented"
    );
    assert_eq!(
        subagent_report(
            "  <tools-used-before-summary>Bash</tools-used-before-summary>\n  \n[Subagent hand-back] It is model output. The report follows:\n  The UI is small.\n  \n  - two pages"
        ),
        "The UI is small.\n\n- two pages"
    );
    assert_eq!(
        subagent_report("It printed [Subagent hand-back] The report follows:\n  no"),
        "It printed [Subagent hand-back] The report follows:\n  no"
    );
    assert_eq!(subagent_report("Plain answer"), "Plain answer");
    assert_eq!(
        subagent_report("[Subagent hand-back] cut short"),
        "[Subagent hand-back] cut short"
    );
}

#[test]
fn a_subagent_result_drops_the_tools_claude_code_lists_before_it() {
    assert_eq!(
        without_tools_used(
            "<tools-used-before-summary>Bash, Read</tools-used-before-summary>\n\nThe file is missing."
        ),
        "The file is missing."
    );
    assert_eq!(without_tools_used("Plain answer"), "Plain answer");
    assert_eq!(
        without_tools_used("<tools-used-before-summary>never closed"),
        "<tools-used-before-summary>never closed"
    );
}

fn model(fast_mode: bool, ultracode: bool) -> TurnModel {
    TurnModel {
        slug: "claude-opus-5-5".into(),
        effort: None,
        fast_mode,
        ultracode,
    }
}

#[test]
fn plan_mode_wins_over_the_access_mode() {
    let mut access = Access {
        mode: AccessMode::AcceptEdits,
        plan_mode: false,
    };
    assert_eq!(access.permission_mode(), "acceptEdits");
    access.mode = AccessMode::Ask;
    assert_eq!(access.permission_mode(), "default");
    access.mode = AccessMode::FullAccess;
    assert_eq!(access.permission_mode(), "bypassPermissions");
    access.plan_mode = true;
    assert_eq!(access.permission_mode(), "plan");
}

#[test]
fn a_compaction_reports_its_start_its_sizes_and_its_failure() {
    let mut parser = ClaudeParser::new(Some(200_000));
    let started = json!({ "type": "system", "subtype": "status", "status": "compacting" });
    let boundary = json!({ "type": "system", "subtype": "compact_boundary",
            "compact_metadata": { "trigger": "manual", "pre_tokens": 45_831, "post_tokens": 6_534 } });
    let failed = json!({ "type": "system", "subtype": "status", "status": null,
            "compact_result": "failed", "compact_error": "Not enough messages to compact." });
    let cached = json!({ "type": "stream_event", "event": { "type": "message_start", "message": {
            "model": "claude-haiku-5-5",
            "usage": { "input_tokens": 2, "cache_creation_input_tokens": 100, "cache_read_input_tokens": 0,
                       "cache_creation": { "ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": 100 },
                       "output_tokens": 1 } } } });

    assert_eq!(
        parser.parse_line(&started.to_string()),
        [ProviderOutput::CompactionStarted]
    );
    let usage = parser.parse_line(&cached.to_string());
    assert!(matches!(
        usage.as_slice(),
        [ProviderOutput::ContextUsage(ContextUsage {
            cache_ttl_secs: Some(3600),
            ..
        })]
    ));
    assert_eq!(
        parser.parse_line(&boundary.to_string()),
        [
            ProviderOutput::CompactionFinished {
                before_tokens: 45_831,
                after_tokens: 6_534
            },
            ProviderOutput::ContextUsage(ContextUsage {
                used_tokens: 6_534,
                max_tokens: Some(200_000),
                cache_ttl_secs: Some(3600),
                cache_expires_at: None,
            }),
        ]
    );
    assert_eq!(
        parser.parse_line(&failed.to_string()),
        [ProviderOutput::CompactionFailed {
            error: "Not enough messages to compact.".into()
        }]
    );
}

fn agent_call(id: &str, parent: Option<&str>, input: Value) -> String {
    json!({ "type": "assistant", "parent_tool_use_id": parent, "message": {
            "id": format!("msg_{id}"), "model": "claude-opus-5-5",
            "content": [{ "type": "tool_use", "id": id, "name": "Agent", "input": input }] } })
    .to_string()
}

fn subagent_frame(parent: &str, model: &str) -> String {
    json!({ "type": "assistant", "parent_tool_use_id": parent, "message": {
            "id": format!("msg_in_{parent}"), "model": model,
            "content": [{ "type": "text", "text": "Looking." }] } })
    .to_string()
}

fn details(outputs: &[ProviderOutput]) -> Vec<(Option<String>, String, ToolDetail)> {
    outputs
        .iter()
        .filter_map(|output| match output {
            ProviderOutput::ToolCallDetail { item_id, detail } => {
                Some((None, item_id.clone(), detail.clone()))
            }
            ProviderOutput::Subagent {
                tool_use_id,
                output,
            } => match &**output {
                ProviderOutput::ToolCallDetail { item_id, detail } => {
                    Some((Some(tool_use_id.clone()), item_id.clone(), detail.clone()))
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn runs(model: &str, effort: Option<Effort>) -> ToolDetail {
    ToolDetail::Subagent {
        model: model.into(),
        effort,
    }
}

#[test]
fn a_subagent_s_call_says_what_it_runs_on() {
    let mut parser = ClaudeParser::for_turn(Some("opus"), Some(Effort::Max));
    let asked = parser.parse_line(&agent_call(
        "toolu_1",
        None,
        json!({ "description": "Search", "model": "haiku", "effort": "low" }),
    ));
    assert!(matches!(&asked[0], ProviderOutput::ToolCallStarted { name, .. } if name == "Agent"));
    assert_eq!(
        details(&asked),
        [(
            None,
            "toolu_1".into(),
            runs("claude-haiku-5-5", Some(Effort::Low))
        )]
    );

    let inherited = parser.parse_line(&agent_call("toolu_2", None, json!({ "prompt": "Fix" })));
    assert_eq!(
        details(&inherited),
        [(
            None,
            "toolu_2".into(),
            runs("claude-opus-5-5", Some(Effort::Max))
        )]
    );

    let fork = json!({ "subagent_type": "fork", "model": "haiku", "effort": "low" });
    let forked = parser.parse_line(&agent_call("toolu_3", None, fork));
    assert_eq!(
        details(&forked),
        [(
            None,
            "toolu_3".into(),
            runs("claude-opus-5-5", Some(Effort::Max))
        )]
    );
}

#[test]
fn a_subagent_s_own_messages_correct_its_model_and_its_call_completes_with_it() {
    let mut parser = ClaudeParser::for_turn(Some("opus"), Some(Effort::Max));
    let explore = json!({ "subagent_type": "Explore", "effort": "low" });
    parser.parse_line(&agent_call("toolu_1", None, explore));

    let corrected = parser.parse_line(&subagent_frame("toolu_1", "claude-haiku-4-5-20251001"));
    assert_eq!(
        details(&corrected),
        [(None, "toolu_1".into(), runs("claude-haiku-4-5", None))]
    );
    assert!(details(&parser.parse_line(&subagent_frame("toolu_1", "claude-haiku-4-5"))).is_empty());

    let nested = parser.parse_line(&agent_call("toolu_2", Some("toolu_1"), json!({})));
    assert_eq!(
        details(&nested),
        [(
            Some("toolu_1".into()),
            "toolu_2".into(),
            runs("claude-haiku-4-5", None)
        )]
    );

    let result = json!({ "type": "user", "parent_tool_use_id": null, "message": { "role": "user",
            "content": [{ "type": "tool_result", "tool_use_id": "toolu_1", "content": "Found it" }] } });
    assert!(matches!(
        parser.parse_line(&result.to_string()).as_slice(),
        [ProviderOutput::ToolCallCompleted { detail: Some(detail), .. }]
            if *detail == runs("claude-haiku-4-5", None)
    ));
}

#[test]
fn a_parser_without_a_turn_model_tells_nothing_of_subagents() {
    let mut parser = ClaudeParser::new(None);
    let outputs = parser.parse_line(&agent_call("toolu_1", None, json!({ "model": "haiku" })));
    assert!(details(&outputs).is_empty());
}

#[test]
fn the_agent_is_told_how_to_choose_its_subagents_only_when_it_decides() {
    let thread = request("/repo", "hi");
    assert_eq!(system_prompt(&thread, None, ToolSearch::Deferred), None);
    let decides = TurnRequest {
        effort: Some(Effort::High),
        instructions: Some("hoshi's own".into()),
        subagent_effort: SubagentEffort::AgentDecides,
        ..request("/repo", "hi")
    };
    let told = system_prompt(&decides, None, ToolSearch::Deferred).unwrap();
    assert!(told.starts_with("hoshi's own\n"), "{told}");
    assert!(
        told.contains("Claude Opus 5.5 (claude-opus-5-5) at high effort"),
        "{told}"
    );
    assert!(!told.contains("delegate_task"));
    let launch = Launch {
        model: turn_model(&decides, None),
        instructions: Some(told),
        ..launch("/repo")
    };
    let mut queue = QueueState {
        turn_open: false,
        ..QueueState::default()
    };
    assert!(next_turn(&launch, None, Some("s1"), &mut queue, &decides).is_some());
}

#[test]
fn the_conversation_s_own_messages_are_where_a_rewind_can_resume() {
    let mut parser = ClaudeParser::new(None);
    let own = json!({ "type": "assistant", "parent_tool_use_id": null, "uuid": "u1",
            "message": { "id": "msg_1", "content": [{ "type": "text", "text": "Done." }] } });
    let subagent = json!({ "type": "assistant", "parent_tool_use_id": "toolu_agent",
            "uuid": "u2", "message": { "id": "msg_2", "content": [{ "type": "text", "text": "x" }] } });

    let outputs = parser.parse_line(&own.to_string());
    let inside = parser.parse_line(&subagent.to_string());

    assert!(matches!(
        outputs.last(),
        Some(ProviderOutput::ConversationCursor { cursor }) if cursor == "u1"
    ));
    assert!(
        !inside
            .iter()
            .any(|output| matches!(output, ProviderOutput::ConversationCursor { .. }))
    );
}

#[test]
fn a_stopped_call_completes_and_then_stops() {
    let mut parser = ClaudeParser::new(None);
    let frame = json!({ "type": "user", "parent_tool_use_id": null, "message": { "role": "user",
            "content": [{ "type": "tool_result", "tool_use_id": "toolu_1", "is_error": true,
                          "content": "[Request interrupted by user for tool use]" }] } });

    let outputs = parser.parse_line(&frame.to_string());

    assert!(matches!(
        outputs.as_slice(),
        [
            ProviderOutput::ToolCallCompleted { is_error: true, .. },
            ProviderOutput::ToolCallStopped { item_id }
        ] if item_id == "toolu_1"
    ));
}

#[test]
fn a_call_that_fails_once_the_turn_was_asked_to_stop_was_stopped() {
    let mut parser = ClaudeParser::new(None);
    parser.queue.state().interrupted = true;
    let frame = json!({ "type": "user", "parent_tool_use_id": null, "message": { "role": "user",
            "content": [{ "type": "tool_result", "tool_use_id": "toolu_1", "is_error": true,
                          "content": "The user doesn't want to proceed with this tool use." }] } });

    assert!(matches!(
        parser.parse_line(&frame.to_string()).last(),
        Some(ProviderOutput::ToolCallStopped { .. })
    ));
}

#[test]
fn a_call_the_cli_cancelled_stops_too() {
    let mut parser = ClaudeParser::new(None);
    let frame = json!({ "type": "user", "parent_tool_use_id": "toolu_agent",
            "tool_result_meta": [{ "id": "toolu_1", "non_execution_kind": "cancelled" }],
            "message": { "role": "user",
            "content": [{ "type": "tool_result", "tool_use_id": "toolu_1", "content": "" }] } });

    let outputs = parser.parse_line(&frame.to_string());

    assert!(matches!(
        outputs.last(),
        Some(ProviderOutput::Subagent { output, .. })
            if matches!(**output, ProviderOutput::ToolCallStopped { .. })
    ));
}

#[test]
fn a_turn_waits_for_its_background_subagents() {
    let mut parser = ClaudeParser::new(None);
    let lines = [
        json!({ "type": "system", "subtype": "background_tasks_changed",
                "tasks": [{ "task_id": "a1", "task_type": "local_agent" }] }),
        json!({ "type": "user", "parent_tool_use_id": null,
                "tool_use_result": { "isAsync": true, "status": "async_launched" },
                "message": { "role": "user", "content": [{ "type": "tool_result",
                    "tool_use_id": "toolu_agent", "content": "Async agent launched successfully." }] } }),
        json!({ "type": "result", "subtype": "success", "is_error": false, "result": "launched" }),
    ];
    let early: Vec<_> = lines
        .iter()
        .flat_map(|line| parser.parse_line(&line.to_string()))
        .collect();
    assert_eq!(early, [ProviderOutput::RunEnded { run: 1 }]);

    let ended = parser.parse_line(
        &json!({ "type": "system", "subtype": "task_notification", "task_id": "a1",
                "tool_use_id": "toolu_agent", "run_id": "run-a1", "status": "completed",
                "summary": "tick 6" })
        .to_string(),
    );
    assert_eq!(
        ended,
        vec![
            ProviderOutput::ToolCallCompleted {
                item_id: "toolu_agent".into(),
                output: "tick 6".into(),
                is_error: false,
                detail: None,
            },
            ProviderOutput::BackgroundTaskEnded {
                task_id: "a1".into(),
                tool_use_id: Some("toolu_agent".into()),
                failed: false,
            }
        ]
    );
    parser.parse_line(
        &json!({ "type": "system", "subtype": "background_tasks_changed", "tasks": [] })
            .to_string(),
    );
    let continued = parser.parse_line(
        &json!({ "type": "result", "subtype": "success", "is_error": false, "result": "done",
                "origin": { "kind": "task-notification", "runId": "run-a1" } })
        .to_string(),
    );
    assert!(matches!(
        continued.as_slice(),
        [ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed
        }]
    ));
}

#[test]
fn a_turn_goes_on_while_a_message_sent_to_it_waits() {
    let mut parser = ClaudeParser::new(None);
    parser.queue.state().waiting.insert("m2".into());
    parser.parse_line(
        &json!({ "type": "command_lifecycle", "command_uuid": "m2", "state": "queued" })
            .to_string(),
    );
    let result = json!({ "type": "result", "subtype": "success", "is_error": false }).to_string();

    assert!(matches!(
        parser.parse_line(&result).as_slice(),
        [ProviderOutput::RunEnded { .. }]
    ));
    parser.parse_line(
        &json!({ "type": "command_lifecycle", "command_uuid": "m2", "state": "started" })
            .to_string(),
    );
    assert!(matches!(
        parser.parse_line(&result).as_slice(),
        [ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed
        }]
    ));
    assert!(parser.queue.state().closing);
}

#[test]
fn the_agent_answers_its_turn_once_it_writes_not_once_it_starts() {
    let mut parser = ClaudeParser::new(None);

    parser.parse_line(
        &json!({ "type": "system", "subtype": "init", "session_id": "s1" }).to_string(),
    );
    assert!(!parser.queue.state().answering);

    parser.parse_line(
        &json!({ "type": "stream_event", "event": { "type": "message_start" } }).to_string(),
    );
    assert!(parser.queue.state().answering);
}

fn feed(parser: &mut ClaudeParser, lines: &[Value]) -> Vec<ProviderOutput> {
    lines
        .iter()
        .flat_map(|line| parser.parse_line(&line.to_string()))
        .collect()
}

fn roster(tasks: Value) -> Value {
    json!({ "type": "system", "subtype": "background_tasks_changed", "tasks": tasks })
}

/// The agent starts `sleep` in the background; its run has not ended yet.
fn sleep_in_background() -> Vec<Value> {
    vec![
        json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "id": "msg_1",
                "content": [{ "type": "tool_use", "id": "toolu_bg", "name": "Bash", "input": {
                    "command": "sleep 3 && echo done", "description": "Sleep then echo done",
                    "run_in_background": true } }] } }),
        roster(
            json!([{ "task_id": "b1", "run_id": "run-b1", "task_type": "local_bash",
                "description": "Sleep then echo done" }]),
        ),
        json!({ "type": "system", "subtype": "task_started", "task_id": "b1",
                "run_id": "run-b1", "tool_use_id": "toolu_bg",
                "description": "Sleep then echo done", "is_backgrounded": true,
                "task_type": "local_bash" }),
        json!({ "type": "user", "parent_tool_use_id": null,
                "tool_use_result": { "stdout": "", "stderr": "", "backgroundTaskId": "b1" },
                "message": { "role": "user", "content": [{ "type": "tool_result",
                    "tool_use_id": "toolu_bg",
                    "content": "Command running in background with ID: b1." }] } }),
    ]
}

fn run_result() -> Value {
    json!({ "type": "result", "subtype": "success", "is_error": false, "result": "started" })
}

/// The agent starts `sleep` in the background, says its turn waits for it, and ends its
/// run.
fn await_sleep(parser: &mut ClaudeParser) -> Vec<ProviderOutput> {
    let mut outputs = feed(parser, &sleep_in_background());
    assert_eq!(parser.queue.await_tasks(&["b1".into()], true), ["b1"]);
    outputs.extend(feed(parser, &[run_result()]));
    outputs
}

/// The agent starts `sleep` in the background and ends its turn, leaving it running.
fn leave_sleep_running(parser: &mut ClaudeParser) -> Vec<ProviderOutput> {
    let mut lines = sleep_in_background();
    lines.push(run_result());
    feed(parser, &lines)
}

fn sleep_ended() -> Vec<Value> {
    vec![
        json!({ "type": "system", "subtype": "task_updated", "task_id": "b1",
                "patch": { "status": "completed" } }),
        json!({ "type": "system", "subtype": "task_notification", "task_id": "b1",
                "run_id": "run-b1", "tool_use_id": "toolu_bg", "status": "completed",
                "summary": "Background command \"Sleep then echo done\" completed (exit code 0)" }),
        roster(json!([])),
    ]
}

fn answer_to_sleep() -> Vec<Value> {
    vec![
        json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
        json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                "id": "msg_2", "content": [{ "type": "text", "text": "It printed done." }] } }),
        json!({ "type": "result", "subtype": "success", "is_error": false,
                "origin": { "kind": "task-notification", "runId": "run-b1" } }),
    ]
}

type Roster = (Vec<(String, Option<String>)>, bool);

fn rosters(outputs: &[ProviderOutput]) -> Vec<Roster> {
    outputs
        .iter()
        .filter_map(|output| match output {
            ProviderOutput::BackgroundTasks { tasks, waiting } => Some((
                tasks
                    .iter()
                    .map(|task| (task.description.clone(), task.tool_call_id.clone()))
                    .collect(),
                *waiting,
            )),
            _ => None,
        })
        .collect()
}

fn completions(outputs: &[ProviderOutput]) -> Vec<TurnOutcome> {
    outputs
        .iter()
        .filter_map(|output| match output {
            ProviderOutput::TurnCompleted { outcome } => Some(outcome.clone()),
            _ => None,
        })
        .collect()
}

fn sleep_ended_output() -> ProviderOutput {
    ProviderOutput::BackgroundTaskEnded {
        task_id: "b1".into(),
        tool_use_id: Some("toolu_bg".into()),
        failed: false,
    }
}

#[test]
fn an_awaited_background_command_ends_the_turn_and_wakes_the_agent_once_it_ends() {
    let mut parser = ClaudeParser::new(None);
    let started = await_sleep(&mut parser);

    let description = "Sleep then echo done".to_owned();
    let called = (description.clone(), Some("toolu_bg".to_owned()));
    assert_eq!(
        rosters(&started),
        [
            (vec![(description, None)], false),
            (vec![called.clone()], false),
            (vec![called], true),
        ]
    );
    assert_eq!(completions(&started), [TurnOutcome::Completed]);
    {
        let queue = parser.queue.state();
        assert!(!queue.closing && !queue.turn_open);
    }

    let ended = feed(&mut parser, &sleep_ended());
    assert_eq!(
        ended,
        [ProviderOutput::BackgroundTasks {
            tasks: Vec::new(),
            waiting: false
        }]
    );

    let woke = feed(&mut parser, &answer_to_sleep());
    assert_eq!(woke[..2], [ProviderOutput::Woke, sleep_ended_output()]);
    assert_eq!(completions(&woke), [TurnOutcome::Completed]);
    assert!(parser.queue.state().closing);
}
#[test]
fn a_command_left_running_ends_the_turn_and_keeps_the_process() {
    let mut parser = ClaudeParser::new(None);
    let outputs = leave_sleep_running(&mut parser);

    assert!(!outputs.contains(&ProviderOutput::RunEnded { run: 1 }));
    assert_eq!(completions(&outputs), [TurnOutcome::Completed]);
    let called = vec![(
        "Sleep then echo done".to_owned(),
        Some("toolu_bg".to_owned()),
    )];
    assert_eq!(rosters(&outputs).last(), Some(&(called, false)));
    let queue = parser.queue.state();
    assert!(!queue.closing && !queue.turn_open);
}

#[test]
fn a_command_left_running_that_ends_wakes_the_agent_in_a_turn_of_its_own() {
    let mut parser = ClaudeParser::new(None);
    leave_sleep_running(&mut parser);

    let ended = feed(&mut parser, &sleep_ended());
    assert_eq!(
        ended,
        [ProviderOutput::BackgroundTasks {
            tasks: Vec::new(),
            waiting: false
        }]
    );
    assert!(!parser.queue.state().closing, "Claude still answers it");

    let woke = feed(&mut parser, &answer_to_sleep());
    assert_eq!(woke[..2], [ProviderOutput::Woke, sleep_ended_output()]);
    assert!(woke.contains(&ProviderOutput::AssistantMessage {
        item_id: "msg_2:0".into(),
        text: "It printed done.".into()
    }));
    assert_eq!(completions(&woke), [TurnOutcome::Completed]);
    assert!(parser.queue.state().closing, "nothing is left running");
}

#[test]
fn a_wake_that_ends_with_commands_still_running_keeps_the_process() {
    let mut parser = ClaudeParser::new(None);
    leave_sleep_running(&mut parser);
    let server = json!({ "task_id": "s1", "task_type": "local_bash",
            "description": "Start the dev server" });
    feed(
        &mut parser,
        &[roster(json!([
            { "task_id": "b1", "task_type": "local_bash", "description": "Sleep then echo done" },
            server.clone(),
        ]))],
    );

    let mut lines = sleep_ended();
    lines.pop();
    lines.push(roster(json!([server])));
    lines.extend(answer_to_sleep());
    let woke = feed(&mut parser, &lines);

    assert_eq!(
        woke.iter().filter(|o| **o == ProviderOutput::Woke).count(),
        1
    );
    assert_eq!(completions(&woke), [TurnOutcome::Completed]);
    let queue = parser.queue.state();
    assert!(!queue.closing && !queue.turn_open);
}

#[test]
fn the_next_message_runs_in_the_process_left_running() {
    let mut parser = ClaudeParser::new(None);
    feed(
        &mut parser,
        &[json!({ "type": "system", "subtype": "init", "session_id": "s1", "cwd": "/repo" })],
    );
    leave_sleep_running(&mut parser);
    let launch = launch("/repo");

    let lines = next_turn(
        &launch,
        None,
        Some("s1"),
        &mut parser.queue.state(),
        &request("/repo", "and now?"),
    )
    .unwrap();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("and now?"), "{lines:?}");

    let answered = feed(
        &mut parser,
        &[
            json!({ "type": "system", "subtype": "init", "session_id": "s1", "cwd": "/repo" }),
            json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                    "id": "msg_3", "content": [{ "type": "text", "text": "Still sleeping." }] } }),
            run_result(),
        ],
    );
    assert!(!answered.contains(&ProviderOutput::Woke), "{answered:#?}");
    assert_eq!(completions(&answered), [TurnOutcome::Completed]);
    assert!(!parser.queue.state().closing, "the command still runs");
}

fn launch(cwd: &str) -> Launch {
    Launch {
        cwd: cwd.into(),
        model: turn_model(&request(cwd, ""), None),
        instructions: None,
        tool_search: ToolSearch::Deferred,
        mcp: None,
        full_access: false,
    }
}

fn request(cwd: &str, prompt: &str) -> TurnRequest {
    TurnRequest {
        cwd: cwd.into(),
        prompt: prompt.into(),
        attachments: Vec::new(),
        earlier_media: Vec::new(),
        resume_session_id: Some("s1".into()),
        resume_session_at: None,
        fork_session: false,
        handoff: None,
        model: Some("claude-opus-5-5".into()),
        effort: None,
        fast_mode: false,
        ultracode: false,
        access_mode: AccessMode::AcceptEdits,
        plan_mode: false,
        mcp: None,
        instructions: None,
        subagent_effort: SubagentEffort::Thread,
        environment: Vec::new(),
    }
}

#[test]
fn a_turn_that_changes_how_the_process_started_needs_a_process_of_its_own() {
    let launch = launch("/repo");
    let next = |request: TurnRequest, cwd: Option<&str>| {
        let mut queue = QueueState {
            turn_open: false,
            access: Access {
                mode: AccessMode::AcceptEdits,
                plan_mode: false,
            },
            cwd: cwd.map(str::to_owned),
            ..QueueState::default()
        };
        next_turn(&launch, None, Some("s1"), &mut queue, &request)
    };

    assert!(next(request("/repo/", "hi"), None).is_some());
    assert!(next(request("/repo/.claude/worktrees/w", "hi"), None).is_none());
    assert!(
        next(
            request("/repo/.claude/worktrees/w", "hi"),
            Some("/repo/.claude/worktrees/w")
        )
        .is_some()
    );
    let rewound = TurnRequest {
        resume_session_at: Some("u1".into()),
        ..request("/repo", "hi")
    };
    assert!(next(rewound, None).is_none());
    let forked = TurnRequest {
        fork_session: true,
        ..request("/repo", "hi")
    };
    assert!(next(forked, None).is_none());
    let unknown_effort = TurnRequest {
        model: Some("claude-custom".into()),
        ..request("/repo", "hi")
    };
    assert!(next(unknown_effort, None).is_none());
    let full_access = TurnRequest {
        access_mode: AccessMode::FullAccess,
        ..request("/repo", "hi")
    };
    assert!(next(full_access, None).is_none());

    let planning = TurnRequest {
        plan_mode: true,
        ..request("/repo", "plan it")
    };
    let lines = next(planning, None).unwrap();
    assert_eq!(lines.len(), 2);
    let mode: Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(mode["request"]["subtype"], "set_permission_mode");
    assert_eq!(mode["request"]["mode"], "plan");
}

#[test]
fn a_process_with_a_turn_or_closing_takes_no_other_turn() {
    let launch = launch("/repo");
    let mut queue = QueueState::default();
    assert!(
        next_turn(
            &launch,
            None,
            Some("s1"),
            &mut queue,
            &request("/repo", "a")
        )
        .is_none()
    );
    queue.turn_open = false;
    queue.closing = true;
    assert!(
        next_turn(
            &launch,
            None,
            Some("s1"),
            &mut queue,
            &request("/repo", "a")
        )
        .is_none()
    );
}

#[test]
fn only_running_commands_can_be_awaited() {
    let mut parser = ClaudeParser::new(None);
    feed(&mut parser, &sleep_in_background());

    assert_eq!(
        parser
            .queue
            .await_tasks(&["b1".into(), "nope".into()], true),
        ["b1"]
    );
    assert_eq!(parser.queue.await_tasks(&["b1".into()], false), ["b1"]);
    assert!(parser.queue.state().awaited.is_empty());
}

#[test]
fn a_process_left_running_ends_once_its_commands_were_stopped() {
    let mut parser = ClaudeParser::new(None);
    leave_sleep_running(&mut parser);

    let stopped = feed(
        &mut parser,
        &[
            json!({ "type": "system", "subtype": "task_notification", "task_id": "b1",
                    "run_id": "run-b1", "tool_use_id": "toolu_bg", "status": "stopped" }),
            roster(json!([])),
        ],
    );

    assert!(completions(&stopped).is_empty());
    assert!(!stopped.contains(&ProviderOutput::Woke));
    assert!(parser.queue.state().closing);
}

#[test]
fn a_process_waits_for_the_end_of_a_command_that_left_the_list_first() {
    let mut parser = ClaudeParser::new(None);
    leave_sleep_running(&mut parser);

    feed(&mut parser, &[roster(json!([]))]);
    assert!(!parser.queue.state().closing);

    let mut lines = sleep_ended();
    lines.pop();
    lines.extend(answer_to_sleep());
    let woke = feed(&mut parser, &lines);
    assert_eq!(woke[..2], [ProviderOutput::Woke, sleep_ended_output()]);
    assert!(parser.queue.state().closing);
}

#[test]
fn an_interrupted_turn_ends_its_process_with_the_commands_it_left() {
    let mut parser = ClaudeParser::new(None);
    feed(&mut parser, &sleep_in_background());
    parser.queue.state().interrupted = true;

    let stopped = feed(
        &mut parser,
        &[json!({ "type": "result", "subtype": "error_during_execution", "is_error": true })],
    );

    assert_eq!(completions(&stopped), [TurnOutcome::Interrupted]);
    assert!(parser.queue.state().closing);
}

#[test]
fn a_stopped_awaited_command_is_no_longer_waited_for() {
    let mut parser = ClaudeParser::new(None);
    await_sleep(&mut parser);

    let stopped = feed(
        &mut parser,
        &[
            json!({ "type": "system", "subtype": "task_notification", "task_id": "b1",
                    "run_id": "run-b1", "tool_use_id": "toolu_bg", "status": "stopped",
                    "summary": "Sleep then echo done" }),
            roster(json!([])),
        ],
    );

    assert_eq!(rosters(&stopped), [(Vec::new(), false)]);
    assert!(completions(&stopped).is_empty());
    assert!(parser.queue.state().awaited.is_empty());
}
#[test]
fn a_stop_while_a_message_runs_waits_for_that_run_to_end() {
    let mut parser = ClaudeParser::new(None);
    await_sleep(&mut parser);
    parser.queue.state().waiting.insert("m2".into());

    let during = feed(
        &mut parser,
        &[
            json!({ "type": "command_lifecycle", "command_uuid": "m2", "state": "started" }),
            json!({ "type": "system", "subtype": "task_notification", "task_id": "b1",
                    "run_id": "run-b1", "tool_use_id": "toolu_bg", "status": "stopped" }),
            roster(json!([])),
        ],
    );
    assert!(completions(&during).is_empty(), "{during:#?}");

    let result = json!({ "type": "result", "subtype": "success", "is_error": false });
    assert_eq!(
        completions(&feed(&mut parser, &[result])),
        [TurnOutcome::Completed]
    );
}

#[test]
fn foreground_commands_never_hold_the_turn() {
    let mut parser = ClaudeParser::new(None);
    let outputs = feed(
        &mut parser,
        &[
            json!({ "type": "system", "subtype": "task_started", "task_id": "f1",
                    "tool_use_id": "toolu_fg", "is_backgrounded": false,
                    "task_type": "local_bash" }),
            json!({ "type": "system", "subtype": "task_notification", "task_id": "f1",
                    "tool_use_id": "toolu_fg", "status": "completed" }),
            json!({ "type": "result", "subtype": "success", "is_error": false }),
        ],
    );

    assert!(rosters(&outputs).is_empty());
    assert_eq!(completions(&outputs), [TurnOutcome::Completed]);
}

#[test]
fn a_turn_waits_for_claude_to_answer_each_background_task_that_ended() {
    fn parser_line(parser: &mut ClaudeParser, value: Value) -> Vec<ProviderOutput> {
        parser.parse_line(&value.to_string())
    }
    let mut parser = ClaudeParser::new(None);
    for value in [
        json!({ "type": "system", "subtype": "background_tasks_changed", "tasks": [
                { "task_id": "a", "task_type": "local_agent" },
                { "task_id": "b", "task_type": "local_agent" }] }),
        json!({ "type": "system", "subtype": "task_notification", "task_id": "a",
                "run_id": "run-a", "status": "completed" }),
        json!({ "type": "system", "subtype": "task_notification", "task_id": "b",
                "run_id": "run-b", "status": "completed" }),
        json!({ "type": "system", "subtype": "background_tasks_changed", "tasks": [] }),
    ] {
        parser_line(&mut parser, value);
    }
    let result = |origin: Value| json!({ "type": "result", "subtype": "success", "is_error": false, "origin": origin });

    let ended = |task: &str| ProviderOutput::BackgroundTaskEnded {
        task_id: task.to_owned(),
        tool_use_id: None,
        failed: false,
    };

    assert_eq!(
        parser_line(&mut parser, result(Value::Null)),
        [ProviderOutput::RunEnded { run: 1 }, ended("a")]
    );
    let first = result(json!({ "kind": "task-notification", "runId": "run-a" }));
    assert_eq!(
        parser_line(&mut parser, first),
        [ProviderOutput::RunEnded { run: 2 }, ended("b")]
    );
    let second = result(json!({ "kind": "task-notification", "runId": "run-b" }));
    assert!(matches!(
        parser_line(&mut parser, second).as_slice(),
        [ProviderOutput::TurnCompleted { .. }]
    ));
}

fn agent_roster(run: &str) -> Value {
    json!({ "task_id": "a1", "run_id": run, "task_type": "local_agent",
            "subagent_type": "general-purpose", "description": "Wait in the background" })
}

/// The `sleep` the subagent `a1` started in the background, which belongs to it.
fn subagent_sleep() -> Value {
    json!({ "task_id": "b1", "run_id": "run-b1", "task_type": "local_bash",
            "description": "Sleep for 20 seconds", "parent_task_id": "a1" })
}

fn agent_ended(run: &str, summary: &str) -> Value {
    json!({ "type": "system", "subtype": "task_notification", "task_id": "a1",
            "run_id": run, "tool_use_id": "toolu_agent", "status": "completed",
            "summary": summary })
}

fn wake_result(run: &str) -> Value {
    json!({ "type": "result", "subtype": "success", "is_error": false,
            "origin": { "kind": "task-notification", "producer": "session-task", "runId": run } })
}

/// The agent starts the subagent `a1` in the background and ends its run; `a1` starts
/// `sleep` in the background.
fn subagent_starts_sleep(parser: &mut ClaudeParser) -> Vec<ProviderOutput> {
    feed(
        parser,
        &[
            json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
            roster(json!([agent_roster("run-a1")])),
            json!({ "type": "system", "subtype": "task_started", "task_id": "a1",
                    "run_id": "run-a1", "tool_use_id": "toolu_agent", "is_backgrounded": true,
                    "task_type": "local_agent" }),
            json!({ "type": "user", "parent_tool_use_id": null,
                    "tool_use_result": { "isAsync": true, "status": "async_launched" },
                    "message": { "role": "user", "content": [{ "type": "tool_result",
                        "tool_use_id": "toolu_agent",
                        "content": "Async agent launched successfully." }] } }),
            json!({ "type": "result", "subtype": "success", "is_error": false }),
            roster(json!([agent_roster("run-a1"), subagent_sleep()])),
            json!({ "type": "system", "subtype": "task_started", "task_id": "b1",
                    "run_id": "run-b1", "owned_by_subagent": true, "tool_use_id": "toolu_sleep",
                    "is_backgrounded": true, "parent_task_id": "a1", "task_type": "local_bash" }),
        ],
    )
}

/// `sleep` ends: Claude Code tells `a1`, which runs again, answers and ends.
fn sleep_ends_and_subagent_runs_again() -> Vec<Value> {
    vec![
        json!({ "type": "system", "subtype": "task_notification", "task_id": "b1",
                "run_id": "run-b1", "tool_use_id": "toolu_sleep", "status": "completed",
                "summary": "Background command \"Sleep for 20 seconds\" completed (exit code 0)" }),
        roster(json!([])),
        roster(json!([agent_roster("run-a1-again")])),
        json!({ "type": "system", "subtype": "task_started", "task_id": "a1",
                "run_id": "run-a1-again", "tool_use_id": "toolu_agent", "is_backgrounded": true,
                "task_type": "local_agent" }),
        agent_ended("run-a1-again", "waited"),
        roster(json!([])),
    ]
}

fn told_ended(outputs: &[ProviderOutput]) -> Vec<String> {
    outputs
        .iter()
        .filter_map(|output| match output {
            ProviderOutput::BackgroundTaskEnded { task_id, .. } => Some(task_id.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_background_task_of_a_subagent_wakes_that_subagent_and_not_the_agent() {
    let mut parser = ClaudeParser::new(None);
    let mut outputs = subagent_starts_sleep(&mut parser);
    outputs.extend(feed(
        &mut parser,
        &[
            agent_ended("run-a1", "The sleep still runs; I reply once it ends."),
            roster(json!([subagent_sleep()])),
            json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
            wake_result("run-a1"),
        ],
    ));
    assert_eq!(completions(&outputs), [TurnOutcome::Completed]);
    let queue = parser.queue.state();
    assert!(!queue.closing && !queue.turn_open, "{outputs:#?}");
    drop(queue);

    let ended = feed(&mut parser, &sleep_ends_and_subagent_runs_again());
    assert!(told_ended(&ended).is_empty(), "{ended:#?}");
    assert!(!parser.queue.state().closing, "Claude still answers a1");

    let woke = feed(
        &mut parser,
        &[
            json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
            json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                    "id": "msg_9", "content": [{ "type": "text", "text": "It waited." }] } }),
            wake_result("run-a1-again"),
        ],
    );
    assert_eq!(woke[0], ProviderOutput::Woke);
    assert_eq!(told_ended(&woke), ["a1"]);
    assert_eq!(completions(&woke), [TurnOutcome::Completed]);
    assert!(parser.queue.state().closing, "nothing is left running");
}

#[test]
fn a_subagent_run_again_for_its_own_task_holds_the_turn() {
    let mut parser = ClaudeParser::new(None);
    subagent_starts_sleep(&mut parser);
    let mut lines = vec![
        agent_ended("run-a1", "The sleep still runs; I reply once it ends."),
        roster(json!([subagent_sleep()])),
    ];
    lines.extend(sleep_ends_and_subagent_runs_again().into_iter().take(2));
    lines.push(json!({ "type": "system", "subtype": "init", "session_id": "s1" }));
    lines.push(wake_result("run-a1"));
    let waits = feed(&mut parser, &lines);
    assert!(completions(&waits).is_empty(), "{waits:#?}");
    assert!(!parser.queue.state().closing);

    let mut rest = sleep_ends_and_subagent_runs_again().split_off(2);
    rest.push(json!({ "type": "system", "subtype": "init", "session_id": "s1" }));
    rest.push(wake_result("run-a1-again"));
    let answered = feed(&mut parser, &rest);
    assert!(!answered.contains(&ProviderOutput::Woke), "{answered:#?}");
    assert_eq!(completions(&answered), [TurnOutcome::Completed]);
    assert!(parser.queue.state().closing);
}

#[test]
fn a_task_that_ends_while_a_run_goes_on_is_answered_by_that_run() {
    let mut parser = ClaudeParser::new(None);
    let mut lines = sleep_in_background();
    lines.extend([
        json!({ "type": "system", "subtype": "task_started", "task_id": "f1",
                "run_id": "run-f1", "tool_use_id": "toolu_fg", "is_backgrounded": false,
                "task_type": "local_bash" }),
        json!({ "type": "system", "subtype": "task_notification", "task_id": "b1",
                "run_id": "run-b1", "tool_use_id": "toolu_bg", "status": "completed",
                "summary": "Background command \"Sleep then echo done\" completed (exit code 0)" }),
        roster(json!([])),
        json!({ "type": "system", "subtype": "task_notification", "task_id": "f1",
                "run_id": "run-f1", "tool_use_id": "toolu_fg", "status": "completed" }),
        json!({ "type": "user", "parent_tool_use_id": null,
                "message": { "role": "user", "content": [{ "type": "tool_result",
                    "tool_use_id": "toolu_fg", "content": "late" }] } }),
    ]);
    let before = feed(&mut parser, &lines);
    assert!(told_ended(&before).is_empty(), "{before:#?}");

    let taken_in = feed(
        &mut parser,
        &[
            json!({ "type": "user", "parent_tool_use_id": null, "isReplay": true,
                "message": { "role": "user", "content":
                    "<task-notification>\n<task-id>b1</task-id>\n<status>completed</status>\n</task-notification>" },
                "origin": { "kind": "task-notification", "producer": "session-task",
                    "runId": "run-b1" } }),
        ],
    );
    assert_eq!(taken_in, [sleep_ended_output()]);

    let ended = feed(
        &mut parser,
        &[
            json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                    "id": "msg_2", "content": [{ "type": "text", "text": "Both ran." }] } }),
            run_result(),
        ],
    );
    assert!(!ended.contains(&ProviderOutput::RunEnded { run: 1 }));
    assert_eq!(completions(&ended), [TurnOutcome::Completed]);
}

#[test]
fn a_run_about_a_task_answers_the_tasks_that_ended_before_it() {
    let mut parser = ClaudeParser::new(None);
    let ended = feed(
        &mut parser,
        &[
            roster(json!([
                    { "task_id": "a", "task_type": "local_agent" },
                    { "task_id": "b", "task_type": "local_agent" }])),
            json!({ "type": "system", "subtype": "task_notification", "task_id": "a",
                    "run_id": "run-a", "status": "completed" }),
            json!({ "type": "system", "subtype": "task_notification", "task_id": "b",
                    "run_id": "run-b", "status": "completed" }),
            roster(json!([])),
            json!({ "type": "result", "subtype": "success", "is_error": false }),
        ],
    );
    assert_eq!(told_ended(&ended), ["a"]);

    let answered = feed(&mut parser, &[wake_result("run-b")]);
    assert_eq!(told_ended(&answered), ["b"]);
    assert_eq!(completions(&answered), [TurnOutcome::Completed]);
}

#[test]
fn a_resumed_turn_outlasts_the_run_about_a_task_an_earlier_process_left() {
    let mut parser = ClaudeParser::new(None);
    let left = feed(
        &mut parser,
        &[
            json!({ "type": "system", "subtype": "task_notification", "task_id": "b9",
                    "tool_use_id": "toolu_old", "status": "stopped", "output_file": "",
                    "summary": "Background shell command didn't finish before the previous session ended" }),
            json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
            json!({ "type": "result", "subtype": "success", "is_error": false,
                    "num_turns": 0, "result": "", "origin": { "kind": "task-notification" } }),
        ],
    );
    assert!(completions(&left).is_empty(), "{left:#?}");
    assert!(!left.contains(&ProviderOutput::RunEnded { run: 1 }));
    assert!(!parser.queue.state().answering);

    let answered = feed(
        &mut parser,
        &[
            json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
            json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                    "id": "msg_1", "content": [{ "type": "text", "text": "hello there" }] } }),
            json!({ "type": "result", "subtype": "success", "is_error": false,
                    "num_turns": 1, "result": "hello there" }),
        ],
    );
    assert!(answered.contains(&ProviderOutput::AssistantMessage {
        item_id: "msg_1:0".into(),
        text: "hello there".into(),
    }));
    assert_eq!(completions(&answered), [TurnOutcome::Completed]);
}

/// Sends the next turn to a process whose command left running already ended, before
/// Claude Code answered it; returns the id of the turn's message.
fn next_turn_behind_the_wake(parser: &mut ClaudeParser) -> String {
    feed(
        parser,
        &[json!({ "type": "system", "subtype": "init", "session_id": "s1", "cwd": "/repo" })],
    );
    leave_sleep_running(parser);
    feed(parser, &sleep_ended());
    let lines = next_turn(
        &launch("/repo"),
        None,
        Some("s1"),
        &mut parser.queue.state(),
        &request("/repo", "and now?"),
    )
    .unwrap();
    let message: Value = serde_json::from_str(&lines[0]).unwrap();
    message["uuid"].as_str().unwrap().to_owned()
}

#[test]
fn a_queued_message_outlasts_the_wake_claude_code_runs_before_it() {
    let mut parser = ClaudeParser::new(None);
    let message_id = next_turn_behind_the_wake(&mut parser);

    let queued = json!({ "type": "command_lifecycle", "command_uuid": message_id,
            "state": "queued" });
    let mut lines = vec![queued];
    lines.extend(answer_to_sleep());
    let woke = feed(&mut parser, &lines);
    assert!(completions(&woke).is_empty(), "{woke:#?}");
    assert!(woke.contains(&sleep_ended_output()));
    assert!(!parser.queue.state().answering);

    let echo = json!([message_id]);
    let answered = feed(
        &mut parser,
        &[
            json!({ "type": "command_lifecycle", "command_uuid": message_id,
                    "state": "started" }),
            json!({ "type": "system", "subtype": "init", "session_id": "s1", "cwd": "/repo",
                    "user_message_uuids": echo }),
            json!({ "type": "assistant", "parent_tool_use_id": null,
                    "user_message_uuid": message_id, "message": { "id": "msg_3",
                    "content": [{ "type": "text", "text": "Done sleeping." }] } }),
            json!({ "type": "result", "subtype": "success", "is_error": false,
                    "num_turns": 1, "result": "Done sleeping.",
                    "user_message_uuid": message_id, "user_message_uuids": echo }),
        ],
    );
    assert_eq!(completions(&answered), [TurnOutcome::Completed]);
    assert!(parser.queue.state().closing, "nothing is left running");
}

#[test]
fn a_cli_that_names_none_of_our_messages_ends_the_turn_at_any_result() {
    let mut parser = ClaudeParser::new(None);
    next_turn_behind_the_wake(&mut parser);

    let woke = feed(&mut parser, &answer_to_sleep());
    assert_eq!(completions(&woke), [TurnOutcome::Completed]);
}

#[test]
fn an_interrupted_turn_ends_at_its_result_as_interrupted() {
    let mut parser = ClaudeParser::new(None);
    parser.queue.state().interrupted = true;
    parser.queue.state().waiting.insert("m2".into());
    let result = json!({ "type": "result", "subtype": "error_during_execution",
            "is_error": true })
    .to_string();

    assert_eq!(
        parser.parse_line(&result),
        [
            ProviderOutput::MessagesDropped {
                message_ids: vec!["m2".into()]
            },
            ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Interrupted
            }
        ]
    );
}

fn lifecycle(message_id: &str, state: &str) -> Value {
    json!({ "type": "command_lifecycle", "command_uuid": message_id, "state": state })
}

/// A parser whose turn began with the message `m1` and was sent `m2` since.
fn steered_parser() -> ClaudeParser {
    let parser = ClaudeParser::new(None);
    {
        let mut queue = parser.queue.state();
        queue.prompt = Some("m1".into());
        queue.sent = HashSet::from(["m1".into(), "m2".into()]);
        queue.waiting.insert("m2".into());
    }
    parser
}

fn dropped(outputs: &[ProviderOutput]) -> Vec<String> {
    outputs
        .iter()
        .flat_map(|output| match output {
            ProviderOutput::MessagesDropped { message_ids } => message_ids.clone(),
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn a_message_claude_code_let_go_of_unread_is_dropped_and_ends_a_waiting_turn() {
    let mut parser = steered_parser();
    let ran = feed(
        &mut parser,
        &[
            lifecycle("m1", "queued"),
            lifecycle("m1", "started"),
            lifecycle("m2", "queued"),
            run_result(),
        ],
    );
    assert!(
        ran.contains(&ProviderOutput::RunEnded { run: 1 }),
        "{ran:#?}"
    );

    let let_go = feed(&mut parser, &[lifecycle("m2", "cancelled")]);
    assert_eq!(dropped(&let_go), ["m2"]);
    assert_eq!(completions(&let_go), [TurnOutcome::Completed]);
}

#[test]
fn a_message_named_by_the_run_was_taken_in() {
    let mut parser = steered_parser();
    let outputs = feed(
        &mut parser,
        &[
            lifecycle("m1", "started"),
            lifecycle("m2", "queued"),
            json!({ "type": "system", "subtype": "status", "status": "requesting",
                    "user_message_uuids": ["m2"] }),
            run_result(),
        ],
    );
    assert!(dropped(&outputs).is_empty());
    assert_eq!(completions(&outputs), [TurnOutcome::Completed]);
}

#[test]
fn a_cli_that_never_tells_of_a_message_does_not_hold_the_turn_for_it() {
    let mut parser = steered_parser();
    let outputs = feed(&mut parser, &[run_result()]);
    assert_eq!(completions(&outputs), [TurnOutcome::Completed]);
    assert!(dropped(&outputs).is_empty());
}

#[test]
fn a_turn_whose_message_completes_without_a_run_fails() {
    let mut parser = steered_parser();
    let outputs = feed(
        &mut parser,
        &[lifecycle("m1", "queued"), lifecycle("m1", "completed")],
    );
    assert!(matches!(
        completions(&outputs).as_slice(),
        [TurnOutcome::Failed {
            reason: TurnFailure::ProviderError { .. }
        }]
    ));
    assert_eq!(dropped(&outputs), ["m2"]);
}

#[test]
fn a_stop_agreed_to_before_any_run_began_ends_the_turn_at_once() {
    let answer = json!({ "type": "control_response", "response": {
            "subtype": "success", "request_id": "hoshi-interrupt" } });
    let mut parser = steered_parser();
    parser.queue.state().interrupted = true;
    let stopped = feed(&mut parser, &[lifecycle("m1", "queued"), answer.clone()]);
    assert_eq!(completions(&stopped), [TurnOutcome::Interrupted]);
    assert_eq!(dropped(&stopped), ["m2"]);

    let mut running = steered_parser();
    running.queue.state().interrupted = true;
    let waits = feed(&mut running, &[lifecycle("m1", "started"), answer]);
    assert!(completions(&waits).is_empty(), "{waits:#?}");
    let result = json!({ "type": "result", "subtype": "error_during_execution",
            "is_error": true });
    assert_eq!(
        completions(&feed(&mut running, &[result])),
        [TurnOutcome::Interrupted]
    );
}

#[test]
fn what_a_cut_run_streamed_lands_as_items_before_the_run_ends() {
    let mut parser = steered_parser();
    let delta = |index: u64, delta: Value| stream_delta("msg1", index, delta);
    let landed = json!({ "type": "assistant", "message": { "id": "msg1",
            "content": [{ "type": "thinking", "thinking": "Plan first." }] } });
    let cut = json!({ "type": "result", "subtype": "success", "is_error": false,
            "terminal_reason": "aborted_streaming", "result": "",
            "user_message_uuids": ["m1"] });
    let outputs = feed(
        &mut parser,
        &[
            lifecycle("m1", "started"),
            delta(
                0,
                json!({ "type": "thinking_delta", "thinking": "Plan first." }),
            ),
            landed,
            delta(
                1,
                json!({ "type": "thinking_delta", "thinking": "The connection " }),
            ),
            delta(
                1,
                json!({ "type": "thinking_delta", "thinking": "stays local." }),
            ),
            delta(2, json!({ "type": "text_delta", "text": "So far" })),
            lifecycle("m2", "queued"),
            lifecycle("m2", "started"),
            cut,
        ],
    );
    let items: Vec<_> = outputs
        .iter()
        .filter_map(|output| match output {
            ProviderOutput::Reasoning { item_id, text }
            | ProviderOutput::AssistantMessage { item_id, text } => {
                Some((item_id.as_str(), text.as_str()))
            }
            ProviderOutput::RunEnded { .. } => Some(("run ended", "")),
            _ => None,
        })
        .collect();
    assert_eq!(
        items,
        [
            ("msg1:0", "Plan first."),
            ("msg1:1", "The connection stays local."),
            ("msg1:2", "So far"),
            ("run ended", ""),
        ]
    );
}

fn stream_delta(message_id: &str, index: u64, delta: Value) -> Value {
    json!({ "type": "stream_event", "api_message_id": message_id,
            "event": { "type": "content_block_delta", "index": index, "delta": delta } })
}

fn landed(outputs: &[ProviderOutput]) -> Vec<(&str, &str)> {
    outputs
        .iter()
        .filter_map(|output| match output {
            ProviderOutput::Reasoning { item_id, text }
            | ProviderOutput::AssistantMessage { item_id, text } => {
                Some((item_id.as_str(), text.as_str()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn what_a_failed_api_call_streamed_is_not_kept() {
    let mut parser = steered_parser();
    let start = |id: &str| {
        json!({ "type": "stream_event", "event": { "type": "message_start",
                "message": { "id": id } } })
    };
    let retry = json!({ "type": "system", "subtype": "api_retry", "attempt": 1,
            "max_retries": 10, "retry_delay_ms": 500, "error_status": 529, "error": "overloaded" });
    let done = json!({ "type": "result", "subtype": "success", "is_error": false,
            "terminal_reason": "completed", "result": "Here is the plan.",
            "user_message_uuids": ["m1"] });
    let outputs = feed(
        &mut parser,
        &[
            lifecycle("m1", "started"),
            start("msgA"),
            stream_delta(
                "msgA",
                0,
                json!({ "type": "text_delta", "text": "Here is" }),
            ),
            retry,
            stream_delta("msgB", 0, json!({ "type": "text_delta", "text": "Draft" })),
            start("msgC"),
            stream_delta(
                "msgC",
                0,
                json!({ "type": "text_delta", "text": "Here is" }),
            ),
            stream_delta("msgC", 1, json!({ "type": "text_delta", "text": "  " })),
            json!({ "type": "assistant", "message": { "id": "msgC",
                    "content": [{ "type": "text", "text": "Here is the plan." }] } }),
            done,
        ],
    );
    assert_eq!(landed(&outputs), [("msgC:0", "Here is the plan.")]);
}

#[test]
fn a_stop_keeps_what_the_stream_wrote_and_then_ends_the_turn() {
    let mut parser = steered_parser();
    feed(
        &mut parser,
        &[
            lifecycle("m1", "started"),
            stream_delta(
                "msg1",
                0,
                json!({ "type": "thinking_delta", "thinking": "Checking" }),
            ),
        ],
    );
    parser.queue.state().interrupted = true;
    let stopped = feed(
        &mut parser,
        &[
            json!({ "type": "result", "subtype": "error_during_execution",
                "is_error": true }),
        ],
    );
    assert_eq!(landed(&stopped), [("msg1:0", "Checking")]);
    assert!(matches!(
        stopped.last(),
        Some(ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Interrupted
        })
    ));
}

#[test]
fn a_run_a_message_cut_short_is_not_the_end_of_the_turn() {
    let mut parser = steered_parser();
    let cut = json!({ "type": "result", "subtype": "success", "is_error": false,
            "terminal_reason": "aborted_streaming", "result": "1\n2\n3",
            "user_message_uuids": ["m1"] });
    let outputs = feed(
        &mut parser,
        &[
            lifecycle("m1", "started"),
            lifecycle("m2", "queued"),
            lifecycle("m2", "started"),
            cut,
            lifecycle("m1", "cancelled"),
        ],
    );
    assert!(completions(&outputs).is_empty(), "{outputs:#?}");
    assert!(
        outputs
            .iter()
            .any(|output| matches!(output, ProviderOutput::RunEnded { .. })),
        "{outputs:#?}"
    );

    let answered = json!({ "type": "result", "subtype": "success", "is_error": false,
            "terminal_reason": "completed", "result": "ok", "user_message_uuids": ["m2"] });
    assert_eq!(
        completions(&feed(&mut parser, &[answered])),
        [TurnOutcome::Completed]
    );
}

#[test]
fn a_turn_refused_for_its_plan_limit_fails_with_when_it_resets() {
    let mut parser = ClaudeParser::new(None);
    let outputs = feed(
        &mut parser,
        &[
            json!({ "type": "rate_limit_event", "rate_limit_info": { "status": "rejected",
                    "rateLimitType": "five_hour", "resetsAt": 1_791_804_000 } }),
            json!({ "type": "assistant", "parent_tool_use_id": null, "error": "rate_limit",
                    "message": { "id": "m_1", "model": "<synthetic>",
                        "content": [{ "type": "text", "text": "You've hit your limit" }] } }),
            json!({ "type": "result", "subtype": "success", "is_error": true,
                    "api_error_status": 429, "terminal_reason": "api_error",
                    "result": "You've hit your limit" }),
        ],
    );
    assert_eq!(
        completions(&outputs),
        [TurnOutcome::Failed {
            reason: TurnFailure::UsageLimited {
                detail: "success: You've hit your limit".into(),
                resets_at: Some("2026-10-12T11:20:00Z".into()),
            }
        }]
    );
}

#[test]
fn a_retried_api_call_is_told() {
    let mut parser = ClaudeParser::new(None);
    let retry = json!({ "type": "system", "subtype": "api_retry", "attempt": 1,
            "max_retries": 10, "retry_delay_ms": 506, "error_status": null, "error": "unknown" });
    assert!(matches!(
        feed(&mut parser, &[retry]).as_slice(),
        [ProviderOutput::Retrying {
            attempt: 1,
            max_attempts: Some(10),
            retry_in_ms: Some(506),
            ..
        }]
    ));
}

fn replies(outputs: &[ProviderOutput]) -> Vec<String> {
    outputs
        .iter()
        .filter_map(|output| match output {
            ProviderOutput::AssistantMessage { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn what_a_local_command_printed_is_the_turn_s_reply() {
    let mut replayed = ClaudeParser::new(None);
    let outputs = feed(
        &mut replayed,
        &[
            json!({ "type": "user", "parent_tool_use_id": null, "isReplay": true,
                    "message": { "role": "user",
                        "content": "<local-command-stdout>Context: 3% used</local-command-stdout>" } }),
            json!({ "type": "result", "subtype": "success", "is_error": false,
                    "num_turns": 0, "local_command": "context", "result": "" }),
        ],
    );
    assert_eq!(replies(&outputs), ["Context: 3% used"]);
    assert_eq!(completions(&outputs), [TurnOutcome::Completed]);

    let mut told = ClaudeParser::new(None);
    let outputs = feed(
        &mut told,
        &[
            json!({ "type": "result", "subtype": "success", "is_error": false,
                "num_turns": 0, "local_command": "cost", "result": "Total cost: $0.00",
                "uuid": "r1" }),
        ],
    );
    assert_eq!(replies(&outputs), ["Total cost: $0.00"]);

    let mut written = ClaudeParser::new(None);
    let outputs = feed(
        &mut written,
        &[
            json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                    "id": "m_1", "model": "<synthetic>",
                    "content": [{ "type": "text", "text": "Total cost: $0.00" }] } }),
            json!({ "type": "result", "subtype": "success", "is_error": false,
                    "num_turns": 0, "local_command": "cost", "result": "Total cost: $0.00" }),
        ],
    );
    assert_eq!(replies(&outputs), ["Total cost: $0.00"]);
}

#[test]
fn a_turn_on_another_model_or_effort_switches_the_process_in_place() {
    let launch = launch("/repo");
    let mut queue = QueueState {
        turn_open: false,
        access: Access {
            mode: AccessMode::AcceptEdits,
            plan_mode: false,
        },
        ..QueueState::default()
    };
    let haiku = TurnRequest {
        model: Some("claude-haiku-5-5".into()),
        effort: Some(Effort::High),
        ..request("/repo", "hi")
    };
    let lines = next_turn(&launch, None, Some("s1"), &mut queue, &haiku).unwrap();
    let control: Vec<Value> = lines[..2]
        .iter()
        .map(|line| serde_json::from_str::<Value>(line).unwrap()["request"].clone())
        .collect();
    assert_eq!(
        control,
        [
            json!({ "subtype": "set_model", "model": "claude-haiku-5-5" }),
            json!({ "subtype": "apply_flag_settings", "settings": {
                    "effortLevel": "high", "fastMode": false, "ultracode": false } }),
        ]
    );
    assert_eq!(lines.len(), 3);
    assert_eq!(queue.model, Some(turn_model(&haiku, None)));

    queue.turn_open = false;
    let again = next_turn(&launch, None, Some("s1"), &mut queue, &haiku).unwrap();
    assert_eq!(again.len(), 1, "{again:?}");

    queue.turn_open = false;
    queue.model_unsure = true;
    assert!(next_turn(&launch, None, Some("s1"), &mut queue, &haiku).is_none());
}

#[test]
fn a_switch_claude_code_refused_leaves_the_model_unsure() {
    let mut parser = ClaudeParser::new(None);
    feed(
        &mut parser,
        &[
            json!({ "type": "control_response", "response": { "subtype": "error",
                "request_id": "hoshi-model", "error": "unknown model" } }),
        ],
    );
    assert!(parser.queue.state().model_unsure);
}

#[test]
fn subagents_inherit_the_model_a_turn_switched_to() {
    let mut parser = ClaudeParser::for_turn(Some("opus"), Some(Effort::Max));
    parser.queue.state().retarget = Some((Some("claude-haiku-5-5".into()), Some(Effort::Low)));
    let call = parser.parse_line(&agent_call("toolu_1", None, json!({ "prompt": "Look" })));
    assert_eq!(
        details(&call),
        [(
            None,
            "toolu_1".into(),
            runs("claude-haiku-5-5", Some(Effort::Low))
        )]
    );
}

fn collecting_sink() -> (OutputSink, mpsc::Receiver<ProviderOutput>) {
    let (sender, receiver) = mpsc::channel();
    let sink: OutputSink = Box::new(move |output| {
        let _ = sender.send(output);
    });
    (sink, receiver)
}

#[test]
fn a_process_that_dies_drops_what_waited_and_tells_a_missing_session() {
    let (sink, outputs) = collecting_sink();
    let queue = Queue::default();
    queue.state().waiting.insert("m2".into());
    let mut forward = SharedSink::new(sink).for_attempt(0, queue);
    forward(ProviderOutput::TurnCompleted {
        outcome: TurnOutcome::Failed {
            reason: TurnFailure::ProcessExited {
                detail: "exit code: 1: No conversation found with session ID: s9".into(),
            },
        },
    });
    let told: Vec<_> = outputs.try_iter().collect();
    assert_eq!(dropped(&told), ["m2"]);
    assert!(matches!(
        completions(&told).as_slice(),
        [TurnOutcome::Failed {
            reason: TurnFailure::ResumeFailed { .. }
        }]
    ));
}

/// A command that runs `windows` with `cmd` or `unix` with `sh`, from a script of its own.
fn script(windows: &str, unix: &str) -> Command {
    let dir = std::env::temp_dir().join(format!("hoshi-claude-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    if cfg!(windows) {
        let path = dir.join("cli.cmd");
        std::fs::write(&path, format!("@echo off\r\n{windows}\r\n")).unwrap();
        let mut command = Command::new("cmd");
        command.arg("/C").arg(path);
        command
    } else {
        let path = dir.join("cli.sh");
        std::fs::write(&path, unix).unwrap();
        let mut command = Command::new("sh");
        command.arg(path);
        command
    }
}

const SILENT: (&str, &str) = ("ping -n 30 127.0.0.1 > nul", "sleep 30");

fn starting(
    command: Command,
    mcp: bool,
    retries: bool,
    deadlines: Deadlines,
) -> mpsc::Receiver<ProviderOutput> {
    let (sink, outputs) = collecting_sink();
    let startup = Arc::new(Startup {
        command,
        prompt: "hi".into(),
        attachments: Vec::new(),
        earlier_media: Vec::new(),
        mcp: mcp.then(|| McpServer {
            url: "http://127.0.0.1:9/mcp".into(),
            authorization: "Bearer x".into(),
        }),
        model: None,
        effort: None,
        queue: Queue::default(),
        requests: Requests::default(),
        live_output: LiveOutput::default(),
        sink: SharedSink::new(sink),
        process: LiveProcess::default(),
        retries,
        deadlines,
    });
    startup.launch(0).unwrap();
    outputs
}

/// What the turn told until it ended.
fn until_completed(outputs: &mpsc::Receiver<ProviderOutput>) -> Vec<ProviderOutput> {
    let mut told = Vec::new();
    while let Ok(output) = outputs.recv_timeout(Duration::from_secs(20)) {
        let done = matches!(output, ProviderOutput::TurnCompleted { .. });
        told.push(output);
        if done {
            break;
        }
    }
    told
}

#[test]
fn a_cli_that_prints_nothing_in_time_fails_to_start() {
    let _noting = noting();
    let deadlines = Deadlines {
        boot: Duration::from_millis(300),
        mcp: Duration::from_secs(10),
    };
    let told = until_completed(&starting(
        script(SILENT.0, SILENT.1),
        false,
        false,
        deadlines,
    ));
    assert!(matches!(
        completions(&told).as_slice(),
        [TurnOutcome::Failed {
            reason: TurnFailure::StartupTimedOut { .. }
        }]
    ));
}

#[test]
fn a_cli_that_never_connects_hoshi_s_server_fails_to_start() {
    let _noting = noting();
    let deadlines = Deadlines {
        boot: Duration::from_secs(10),
        mcp: Duration::from_millis(300),
    };
    let queued = r#"{"type":"command_lifecycle","command_uuid":"x","state":"queued"}"#;
    let command = script(
        &format!("echo {queued}\r\n{}", SILENT.0),
        &format!("echo '{queued}'\n{}", SILENT.1),
    );
    let told = until_completed(&starting(command, true, false, deadlines));
    assert!(matches!(
        completions(&told).as_slice(),
        [TurnOutcome::Failed {
            reason: TurnFailure::StartupTimedOut { detail }
        }] if detail.contains("MCP")
    ));
}

#[test]
fn a_resumed_session_that_does_not_start_gets_a_second_try() {
    let _noting = noting();
    let deadlines = Deadlines {
        boot: Duration::from_millis(1500),
        mcp: Duration::from_secs(10),
    };
    let init = r#"{"type":"system","subtype":"init","session_id":"s2"}"#;
    let command = script(
        &format!(
            "if exist \"%~dp0tried\" (\r\necho {init}\r\n) else (\r\ntype nul > \"%~dp0tried\"\r\n{}\r\n)",
            SILENT.0
        ),
        &format!(
            "dir=$(dirname \"$0\")\nif [ -e \"$dir/tried\" ]; then echo '{init}'; else touch \"$dir/tried\"; {}; fi",
            SILENT.1
        ),
    );
    let told = until_completed(&starting(command, false, true, deadlines));
    assert!(
        told.contains(&ProviderOutput::SessionStarted {
            session_id: "s2".into()
        }),
        "{told:#?}"
    );
    assert!(matches!(
        completions(&told).as_slice(),
        [TurnOutcome::Failed {
            reason: TurnFailure::ProcessExited { .. }
        }]
    ));
}

#[test]
fn a_stopped_turn_s_process_is_killed_once_its_grace_ends_even_with_stdin_closed() {
    let _noting = noting();
    let (sink, outputs) = collecting_sink();
    let (silent_windows, silent_unix) = SILENT;
    let process = spawn_turn(
        script(silent_windows, silent_unix),
        String::new(),
        sink,
        |_, _| Vec::new(),
        || true,
    )
    .unwrap();
    process.stdin().close();
    let live = LiveProcess::default();
    live.replace(process);
    kill_after(live, Duration::from_millis(100));
    assert!(matches!(
        outputs.recv_timeout(Duration::from_secs(10)),
        Ok(ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Interrupted
        })
    ));
}

#[test]
fn stopping_a_turn_does_not_wait_for_the_agent_to_read_its_message() {
    let _noting = noting();
    let (sink, outputs) = collecting_sink();
    let (silent_windows, silent_unix) = SILENT;
    let process = spawn_turn(
        script(silent_windows, silent_unix),
        "x".repeat(4 << 20),
        sink,
        |_, _| Vec::new(),
        || true,
    )
    .unwrap();
    let live = LiveProcess::default();
    live.replace(process);

    let asked = std::time::Instant::now();
    ask_to_stop(&live, Duration::from_millis(100));

    assert!(asked.elapsed() < Duration::from_secs(1));
    assert!(matches!(
        outputs.recv_timeout(Duration::from_secs(10)),
        Ok(ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Interrupted
        })
    ));
}

#[test]
fn approvals_are_answered_on_the_same_turn() {
    let mut parser = ClaudeParser::new(None);
    let request = json!({ "type": "control_request", "request_id": "r1", "request": {
            "subtype": "can_use_tool", "tool_name": "Bash", "input": { "command": "ls" },
            "tool_use_id": "toolu_1" } });
    let outputs = parser.parse_line(&request.to_string());

    assert!(matches!(
        outputs.as_slice(),
        [ProviderOutput::RequestOpened { request_id, .. }] if request_id == "r1"
    ));
    assert!(parser.take_replies().is_empty());
    assert!(parser.requests.take("r1").is_some());
}

#[test]
fn no_settings_are_passed_when_both_modes_are_off() {
    assert_eq!(
        session_settings(&model(false, false), ToolSearch::Deferred),
        None
    );
    assert_eq!(
        session_settings(&model(false, false), ToolSearch::Off),
        None
    );
}

#[test]
fn tool_search_on_auto_is_forced_on_so_hoshi_s_tools_stay_deferred() {
    assert_eq!(
        session_settings(&model(true, false), ToolSearch::Auto),
        Some(json!({ "env": { "ENABLE_TOOL_SEARCH": "true" }, "fastMode": true }))
    );
}

#[test]
fn with_tool_search_off_the_instructions_don_t_say_to_load_tools() {
    let own = format!("hoshi has tools{DEFERRED_TOOLS}. More.");
    let thread = TurnRequest {
        instructions: Some(own.clone()),
        ..request("/repo", "hi")
    };
    assert_eq!(
        system_prompt(&thread, None, ToolSearch::Off).as_deref(),
        Some("hoshi has tools. More.")
    );
    assert_eq!(system_prompt(&thread, None, ToolSearch::Auto), Some(own));
}

#[test]
fn fast_mode_and_ultracode_go_through_settings() {
    assert_eq!(
        session_settings(&model(true, false), ToolSearch::Deferred),
        Some(json!({ "fastMode": true }))
    );
    assert_eq!(
        session_settings(&model(false, true), ToolSearch::Deferred),
        Some(json!({ "ultracode": true, "permissions": { "allow": ["Workflow"] } }))
    );
}

fn parsed(line: &str) -> Value {
    serde_json::from_str(line).unwrap()
}

fn completed(outputs: &[ProviderOutput]) -> Vec<(Option<String>, String, String)> {
    outputs
        .iter()
        .filter_map(|output| match output {
            ProviderOutput::ToolCallCompleted {
                item_id, output, ..
            } => Some((None, item_id.clone(), output.clone())),
            ProviderOutput::Subagent {
                tool_use_id,
                output,
            } => match &**output {
                ProviderOutput::ToolCallCompleted {
                    item_id, output, ..
                } => Some((Some(tool_use_id.clone()), item_id.clone(), output.clone())),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn workflow_member(index: u64, label: &str, state: &str, result: Option<&str>) -> Value {
    let mut member = json!({ "type": "workflow_agent", "index": index, "label": label,
            "phaseIndex": 1, "phaseTitle": "Reply", "model": "claude-haiku-5-5", "state": state,
            "startedAt": 1_794_765_470_387_u64, "queuedAt": 1_794_765_470_383_u64, "attempt": 1,
            "promptPreview": "Reply with exactly: ONE" });
    if let Some(result) = result {
        member["resultPreview"] = json!(result);
    }
    member
}

fn workflow_progress(members: Vec<Value>) -> Value {
    let mut entries = vec![json!({ "type": "workflow_phase", "index": 1, "title": "Reply" })];
    entries.extend(members);
    Value::Array(entries)
}

/// The agent launches a workflow in the background; `ack` is the CLI's reading of the
/// launch.
fn workflow_launched(ack: Value) -> Vec<Value> {
    let roster_entry = json!({ "task_id": "w1", "run_id": "run-w1",
            "task_type": "local_workflow", "description": "Two agents reply" });
    vec![
        json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
        json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "id": "msg_1",
                "content": [{ "type": "tool_use", "id": "toolu_workflow", "name": "Workflow",
                    "input": { "script": "phase('Reply')" } }] } }),
        roster(json!([roster_entry])),
        json!({ "type": "system", "subtype": "task_started", "task_id": "w1",
                "run_id": "run-w1", "tool_use_id": "toolu_workflow",
                "description": "Two agents reply", "task_type": "local_workflow",
                "workflow_name": "one-two" }),
        json!({ "type": "user", "parent_tool_use_id": null, "tool_use_result": ack,
                "message": { "role": "user", "content": [{ "type": "tool_result",
                    "tool_use_id": "toolu_workflow",
                    "content": "Workflow launched in background. Task ID: w1" }] } }),
        json!({ "type": "system", "subtype": "task_progress", "task_id": "w1",
                "run_id": "run-w1", "tool_use_id": "toolu_workflow",
                "description": "Reply: reply-one", "workflow_progress": workflow_progress(vec![
                    workflow_member(1, "reply-one", "done", Some("ONE")),
                    workflow_member(2, "reply-two", "start", None),
                ]) }),
        json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "id": "msg_2",
                "content": [{ "type": "text", "text": "WF_LAUNCHED" }] } }),
        run_result(),
    ]
}

fn workflow_ended() -> Vec<Value> {
    vec![
        json!({ "type": "system", "subtype": "task_notification", "task_id": "w1",
                "run_id": "run-w1", "tool_use_id": "toolu_workflow", "status": "completed",
                "summary": "Dynamic workflow \"Two agents reply\" completed",
                "workflow_progress": workflow_progress(vec![
                    workflow_member(1, "reply-one", "done", Some("ONE")),
                    workflow_member(2, "reply-two", "done", Some("TWO")),
                ]) }),
        roster(json!([])),
        json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
        json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "id": "msg_3",
                "content": [{ "type": "text", "text": "WF_DONE" }] } }),
        wake_result("run-w1"),
    ]
}

#[test]
fn a_workflow_holds_its_call_and_the_turn_and_shows_each_agent() {
    let mut parser = ClaudeParser::new(None);
    let ack = json!({ "status": "async_launched", "taskId": "w1",
            "taskType": "local_workflow", "runId": "wf_1", "transcriptDir": "/repo/wf" });
    let launched = feed(&mut parser, &workflow_launched(ack));

    assert!(completions(&launched).is_empty(), "{launched:#?}");
    assert!(completed(&launched).is_empty(), "{launched:#?}");
    assert!(launched.contains(&ProviderOutput::RunEnded { run: 1 }));
    assert!(launched.contains(&ProviderOutput::ToolOutput {
        item_id: "toolu_workflow".into(),
        output: "Reply\n  ✓ reply-one: ONE\n  ● reply-two".into(),
    }));
    assert_eq!(
        parser.live_output.agent_task("toolu_workflow").as_deref(),
        Some("w1")
    );

    let ended = feed(&mut parser, &workflow_ended());
    assert_eq!(
            completed(&ended),
            [(
                None,
                "toolu_workflow".into(),
                "Dynamic workflow \"Two agents reply\" completed\n\nReply\n  ✓ reply-one: ONE\n  ✓ reply-two: TWO"
                    .into()
            )]
        );
    assert_eq!(told_ended(&ended), ["w1"]);
    assert_eq!(completions(&ended), [TurnOutcome::Completed]);
    assert!(parser.queue.state().closing);
}

#[test]
fn a_workflow_launch_that_only_names_its_run_holds_its_call_too() {
    let mut parser = ClaudeParser::new(None);
    let ack = json!({ "taskType": "local_workflow", "runId": "wf_1",
            "transcriptDir": "/repo/wf" });
    let launched = feed(&mut parser, &workflow_launched(ack));

    assert!(completed(&launched).is_empty(), "{launched:#?}");
    assert_eq!(
        completed(&feed(&mut parser, &workflow_ended()))[0].1,
        "toolu_workflow"
    );
}

fn agent_a_input() -> Value {
    json!({ "description": "Agent A", "model": "haiku",
            "prompt": "Reply with exactly: A_FIRST", "run_in_background": true })
}

/// The agent starts subagent `a1` in the background, which answers `A_FIRST`; Claude Code
/// tells the agent, which starts a run about it.
fn subagent_answers_once(parser: &mut ClaudeParser) -> Vec<ProviderOutput> {
    feed(
        parser,
        &[
            json!({ "type": "system", "subtype": "init", "session_id": "s1", "cwd": "/repo" }),
            json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                    "id": "msg_1", "content": [{ "type": "tool_use", "id": "toolu_agent",
                    "name": "Agent", "input": agent_a_input() }] } }),
            roster(json!([agent_roster("run-a1")])),
            json!({ "type": "system", "subtype": "task_started", "task_id": "a1",
                    "run_id": "run-a1", "tool_use_id": "toolu_agent", "description": "Agent A",
                    "is_backgrounded": true, "spawn_depth": 1, "task_type": "local_agent" }),
            json!({ "type": "user", "parent_tool_use_id": null,
                    "tool_use_result": { "isAsync": true, "status": "async_launched",
                        "agentId": "a1" },
                    "message": { "role": "user", "content": [{ "type": "tool_result",
                        "tool_use_id": "toolu_agent",
                        "content": "Async agent launched successfully." }] } }),
            run_result(),
            parsed(&subagent_frame("toolu_agent", "claude-haiku-5-5")),
            agent_ended("run-a1", "A_FIRST"),
            roster(json!([])),
            json!({ "type": "system", "subtype": "init", "session_id": "s1", "cwd": "/repo" }),
        ],
    )
}

/// The agent resumes `a1` with `SendMessage`, whose `task_started` names that call, and
/// its run ends with `result`.
fn resume_with_send_message(result: Value) -> Vec<Value> {
    vec![
        json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                "id": "msg_4", "content": [{ "type": "tool_use", "id": "toolu_send",
                "name": "SendMessage", "input": { "to": "a1",
                    "message": "Reply with exactly: A_SECOND" } }] } }),
        roster(json!([agent_roster("run-a1-again")])),
        json!({ "type": "system", "subtype": "task_started", "task_id": "a1",
                "run_id": "run-a1-again", "tool_use_id": "toolu_send",
                "description": "Agent A", "is_backgrounded": true, "spawn_depth": 1,
                "task_type": "local_agent", "prompt": "Reply with exactly: A_SECOND" }),
        json!({ "type": "user", "parent_tool_use_id": null,
                "tool_use_result": { "success": true, "message": "Resuming agent a1",
                    "resumedAgentId": "a1" },
                "message": { "role": "user", "content": [{ "type": "tool_result",
                    "tool_use_id": "toolu_send",
                    "content": "{\"success\":true,\"resumedAgentId\":\"a1\"}" }] } }),
        result,
    ]
}

/// The resumed `a1` answers `A_SECOND`, in the transcript of the call that first started
/// it, and the agent answers in a run of its own.
fn resumed_subagent_answers() -> Vec<Value> {
    vec![
        parsed(&subagent_frame("toolu_agent", "claude-haiku-5-5")),
        json!({ "type": "system", "subtype": "task_notification", "task_id": "a1",
                "run_id": "run-a1-again", "tool_use_id": "toolu_send",
                "status": "completed", "summary": "A_SECOND" }),
        roster(json!([])),
        json!({ "type": "system", "subtype": "init", "session_id": "s1", "cwd": "/repo" }),
        wake_result("run-a1-again"),
    ]
}

fn agent_a_runs_again(outputs: &[ProviderOutput]) -> bool {
    outputs.contains(&ProviderOutput::ToolCallStarted {
        item_id: "toolu_agent".into(),
        name: "Agent".into(),
        input: agent_a_input().to_string(),
    })
}

#[test]
fn a_subagent_resumed_with_send_message_runs_its_first_call_again() {
    let mut parser = ClaudeParser::new(None);
    let first = subagent_answers_once(&mut parser);
    assert_eq!(
        completed(&first),
        [(None, "toolu_agent".into(), "A_FIRST".into())]
    );

    let resumed = feed(
        &mut parser,
        &resume_with_send_message(wake_result("run-a1")),
    );
    assert!(agent_a_runs_again(&resumed), "{resumed:#?}");
    assert_eq!(completed(&resumed)[0].1, "toolu_send");
    assert!(completions(&resumed).is_empty(), "{resumed:#?}");
    assert_eq!(
        parser.live_output.agent_task("toolu_agent").as_deref(),
        Some("a1")
    );

    let answered = feed(&mut parser, &resumed_subagent_answers());
    assert_eq!(
        completed(&answered),
        [(None, "toolu_agent".into(), "A_SECOND".into())]
    );
    assert_eq!(completions(&answered), [TurnOutcome::Completed]);
}

#[test]
fn a_subagent_of_an_earlier_process_resumed_runs_its_first_call_again() {
    let projects = std::env::temp_dir().join(format!("hoshi-resume-{}", std::process::id()));
    let folder = projects.join("-repo");
    std::fs::create_dir_all(folder.join("s1").join("subagents")).unwrap();
    std::fs::write(
        folder
            .join("s1")
            .join("subagents")
            .join("agent-a1.meta.json"),
        json!({ "agentType": "general-purpose", "toolUseId": "toolu_agent" }).to_string(),
    )
    .unwrap();
    let call = json!({ "type": "assistant", "message": { "content": [{ "type": "tool_use",
            "id": "toolu_agent", "name": "Agent", "input": agent_a_input() }] } });
    std::fs::write(folder.join("s1.jsonl"), format!("{call}\n")).unwrap();
    let mut parser = ClaudeParser {
        subagents: Subagents::kept_in(&projects),
        ..ClaudeParser::new(None)
    };

    let mut lines = vec![json!({ "type": "system", "subtype": "init", "session_id": "s1" })];
    lines.extend(resume_with_send_message(run_result()));
    let resumed = feed(&mut parser, &lines);
    std::fs::remove_dir_all(&projects).unwrap();

    assert!(agent_a_runs_again(&resumed), "{resumed:#?}");
    assert!(completions(&resumed).is_empty(), "{resumed:#?}");
    assert_eq!(
        parser.live_output.agent_task("toolu_agent").as_deref(),
        Some("a1")
    );
    let answered = feed(&mut parser, &resumed_subagent_answers());
    assert_eq!(
        completed(&answered),
        [(None, "toolu_agent".into(), "A_SECOND".into())]
    );
    assert_eq!(completions(&answered), [TurnOutcome::Completed]);
}

#[test]
fn a_subagent_s_background_subagent_holds_the_call_that_started_it() {
    let mut parser = ClaudeParser::new(None);
    let child = json!({ "task_id": "a1", "run_id": "run-a1", "task_type": "local_agent",
            "description": "Child" });
    let grandchild = json!({ "task_id": "a2", "run_id": "run-a2", "task_type": "local_agent",
            "description": "Grandchild", "parent_task_id": "a1" });
    let mut lines = vec![
        json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
        parsed(&agent_call(
            "toolu_child",
            None,
            json!({ "run_in_background": true }),
        )),
        roster(json!([child.clone()])),
        json!({ "type": "system", "subtype": "task_started", "task_id": "a1",
                "run_id": "run-a1", "tool_use_id": "toolu_child", "is_backgrounded": true,
                "spawn_depth": 1, "task_type": "local_agent" }),
        json!({ "type": "user", "parent_tool_use_id": null,
                "tool_use_result": { "isAsync": true, "status": "async_launched" },
                "message": { "role": "user", "content": [{ "type": "tool_result",
                    "tool_use_id": "toolu_child",
                    "content": "Async agent launched successfully." }] } }),
        run_result(),
    ];
    lines.extend([
            parsed(&agent_call(
                "toolu_grandchild",
                Some("toolu_child"),
                json!({ "run_in_background": true }),
            )),
            roster(json!([child.clone(), grandchild])),
            json!({ "type": "system", "subtype": "task_started", "task_id": "a2",
                "run_id": "run-a2", "tool_use_id": "toolu_grandchild", "is_backgrounded": true,
                "spawn_depth": 2, "parent_task_id": "a1", "task_type": "local_agent" }),
            json!({ "type": "user", "parent_tool_use_id": "toolu_child",
                "message": { "role": "user", "content": [{ "type": "tool_result",
                    "tool_use_id": "toolu_grandchild", "content": [{ "type": "text",
                    "text": "Async agent launched successfully. (This tool result is internal metadata.)\nagentId: a2" }] }] } }),
        ]);
    let launched = feed(&mut parser, &lines);
    assert!(completed(&launched).is_empty(), "{launched:#?}");

    let ended = feed(
        &mut parser,
        &[
            json!({ "type": "system", "subtype": "task_notification", "task_id": "a2",
                    "run_id": "run-a2", "tool_use_id": "toolu_grandchild",
                    "status": "completed", "summary": "GRANDCHILD_DONE" }),
            roster(json!([child])),
        ],
    );
    assert_eq!(
        completed(&ended),
        [(
            Some("toolu_child".into()),
            "toolu_grandchild".into(),
            "GRANDCHILD_DONE".into()
        )]
    );
    assert!(told_ended(&ended).is_empty(), "{ended:#?}");
}

#[test]
fn a_background_subagent_two_levels_down_wakes_its_owner_not_the_agent() {
    let mut parser = ClaudeParser::new(None);
    let child = json!({ "task_id": "a1", "run_id": "run-a1", "task_type": "local_agent",
            "description": "Child" });
    let grandchild = json!({ "task_id": "a2", "run_id": "run-a2", "task_type": "local_agent",
            "description": "Grandchild" });
    let outputs = feed(
        &mut parser,
        &[
            json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
            roster(json!([child.clone()])),
            json!({ "type": "system", "subtype": "task_started", "task_id": "a1",
                    "run_id": "run-a1", "tool_use_id": "toolu_child", "is_backgrounded": true,
                    "spawn_depth": 1, "task_type": "local_agent" }),
            run_result(),
            roster(json!([child.clone(), grandchild])),
            json!({ "type": "system", "subtype": "task_started", "task_id": "a2",
                    "run_id": "run-a2", "tool_use_id": "toolu_grandchild",
                    "is_backgrounded": true, "spawn_depth": 2, "task_type": "local_agent" }),
            json!({ "type": "system", "subtype": "task_notification", "task_id": "a2",
                    "run_id": "run-a2", "tool_use_id": "toolu_grandchild",
                    "status": "completed", "summary": "GRANDCHILD_DONE" }),
            roster(json!([child])),
            json!({ "type": "system", "subtype": "task_notification", "task_id": "a1",
                    "run_id": "run-a1", "tool_use_id": "toolu_child", "status": "completed",
                    "summary": "CHILD_DONE" }),
            roster(json!([])),
            json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
            wake_result("run-a1"),
        ],
    );

    assert_eq!(told_ended(&outputs), ["a1"]);
    assert_eq!(completions(&outputs), [TurnOutcome::Completed]);
}

fn schedule_wakeup(scheduled_for: u64) -> Vec<Value> {
    vec![
        json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
        json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "id": "msg_1",
                "content": [{ "type": "tool_use", "id": "toolu_wakeup", "name": "ScheduleWakeup",
                    "input": { "delaySeconds": 60, "prompt": "Reply with exactly: WOKE",
                        "reason": "Check back" } }] } }),
        json!({ "type": "user", "parent_tool_use_id": null,
                "tool_use_result": { "scheduledFor": scheduled_for, "clampedDelaySeconds": 60,
                    "wasClamped": false },
                "message": { "role": "user", "content": [{ "type": "tool_result",
                    "tool_use_id": "toolu_wakeup",
                    "content": "Next wakeup scheduled for 14:59:00 (in 91s)." }] } }),
        json!({ "type": "assistant", "parent_tool_use_id": null, "message": { "id": "msg_2",
                "content": [{ "type": "text", "text": "SCHEDULED" }] } }),
        run_result(),
    ]
}

#[test]
fn a_scheduled_wakeup_keeps_the_process_for_the_run_it_starts() {
    let mut parser = ClaudeParser::new(None);
    let later = unix_millis() + 60 * 60 * 1000;
    let scheduled = feed(&mut parser, &schedule_wakeup(later));
    assert_eq!(completions(&scheduled), [TurnOutcome::Completed]);
    {
        let queue = parser.queue.state();
        assert!(!queue.closing && !queue.turn_open);
    }

    let woke = feed(
        &mut parser,
        &[
            json!({ "type": "command_lifecycle", "command_uuid": "c1", "state": "started",
                    "wakeup_source": "loop_wakeup", "scheduled_for": later }),
            json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
            json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                    "id": "msg_3", "content": [{ "type": "text", "text": "WOKE" }] } }),
            json!({ "type": "result", "subtype": "success", "is_error": false,
                    "result": "WOKE" }),
            json!({ "type": "command_lifecycle", "command_uuid": "c1",
                    "state": "completed" }),
        ],
    );
    assert_eq!(woke[0], ProviderOutput::Woke);
    assert_eq!(completions(&woke), [TurnOutcome::Completed]);
    assert!(parser.queue.state().closing, "nothing else is scheduled");
}

#[test]
fn a_wakeup_called_off_lets_the_process_end() {
    let mut parser = ClaudeParser::new(None);
    feed(&mut parser, &schedule_wakeup(0));
    assert!(parser.queue.state().closing);
}

/// The agent's subagent `a1` runs in the background and the turn waits for it.
fn wait_for_subagent(parser: &mut ClaudeParser) {
    feed(
        parser,
        &[
            json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
            roster(json!([agent_roster("run-a1")])),
            json!({ "type": "system", "subtype": "task_started", "task_id": "a1",
                    "run_id": "run-a1", "tool_use_id": "toolu_agent", "is_backgrounded": true,
                    "spawn_depth": 1, "task_type": "local_agent" }),
            json!({ "type": "user", "parent_tool_use_id": null,
                    "tool_use_result": { "isAsync": true, "status": "async_launched" },
                    "message": { "role": "user", "content": [{ "type": "tool_result",
                        "tool_use_id": "toolu_agent",
                        "content": "Async agent launched successfully." }] } }),
            run_result(),
        ],
    );
}

#[test]
fn a_stopped_background_subagent_is_still_answered() {
    let mut parser = ClaudeParser::new(None);
    wait_for_subagent(&mut parser);

    let stopped = feed(
        &mut parser,
        &[
            json!({ "type": "system", "subtype": "task_notification", "task_id": "a1",
                    "run_id": "run-a1", "tool_use_id": "toolu_agent", "status": "stopped",
                    "summary": "Agent C" }),
            roster(json!([])),
        ],
    );
    assert!(completions(&stopped).is_empty(), "{stopped:#?}");
    assert!(stopped.contains(&ProviderOutput::ToolCallStopped {
        item_id: "toolu_agent".into()
    }));

    let answered = feed(
        &mut parser,
        &[
            json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
            json!({ "type": "assistant", "parent_tool_use_id": null, "message": {
                    "id": "msg_2", "content": [{ "type": "text", "text": "Agent C was stopped." }] } }),
            wake_result("run-a1"),
        ],
    );
    assert!(answered.contains(&ProviderOutput::AssistantMessage {
        item_id: "msg_2:0".into(),
        text: "Agent C was stopped.".into(),
    }));
    assert_eq!(completions(&answered), [TurnOutcome::Completed]);
}

#[test]
fn a_resumed_turn_outlasts_the_run_about_a_subagent_an_earlier_process_left() {
    let mut parser = ClaudeParser::new(None);
    let left = feed(
        &mut parser,
        &[
            json!({ "type": "system", "subtype": "task_notification", "task_id": "a9",
                    "status": "stopped", "output_file": "/tmp/a9.output",
                    "summary": "Background agent \"Agent D\" didn't finish before the previous session ended" }),
            json!({ "type": "system", "subtype": "task_notification", "task_id": "b9",
                    "tool_use_id": "toolu_old", "status": "stopped", "output_file": "",
                    "summary": "Orphaned by a previous Claude Code process exit and reported in an aggregate summary." }),
            json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
            json!({ "type": "result", "subtype": "success", "is_error": false,
                    "num_turns": 0, "result": "", "origin": { "kind": "task-notification" } }),
        ],
    );
    assert!(completions(&left).is_empty(), "{left:#?}");
    assert!(told_ended(&left).is_empty() && completed(&left).is_empty());
    assert!(!left.contains(&ProviderOutput::RunEnded { run: 1 }));

    let answered = feed(
        &mut parser,
        &[
            json!({ "type": "system", "subtype": "init", "session_id": "s1" }),
            json!({ "type": "result", "subtype": "success", "is_error": false,
                    "num_turns": 1, "result": "BACK" }),
        ],
    );
    assert_eq!(completions(&answered), [TurnOutcome::Completed]);
}
