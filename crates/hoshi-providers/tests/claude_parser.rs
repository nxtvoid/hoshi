use hoshi_core::{ContextUsage, ProviderOutput, TurnOutcome, UsageWindowKind};
use hoshi_providers::ClaudeParser;

const READ_TURN: &str = include_str!("fixtures/claude-read-turn.jsonl");
const THINKING_TURN: &str = include_str!("fixtures/claude-thinking-turn.jsonl");

fn parse(fixture: &str) -> Vec<ProviderOutput> {
    let mut parser = ClaudeParser::default();
    fixture
        .lines()
        .flat_map(|line| parser.parse_line(line))
        .collect()
}

#[test]
fn a_turn_with_a_tool_call_becomes_session_tool_and_reply() {
    let outputs: Vec<_> = parse(READ_TURN)
        .into_iter()
        .filter(|output| {
            !matches!(
                output,
                ProviderOutput::TextDelta { .. }
                    | ProviderOutput::ReasoningDelta { .. }
                    | ProviderOutput::ContextUsage(_)
                    | ProviderOutput::UsageLimits(_)
            )
        })
        .collect();

    let [
        ProviderOutput::SessionStarted { session_id },
        ProviderOutput::WorkingDirectory { path: cwd },
        ProviderOutput::ToolCallStarted {
            item_id: tool_id,
            name,
            input,
        },
        ProviderOutput::ToolCallCompleted {
            item_id: result_id,
            output,
            is_error,
            ..
        },
        ProviderOutput::AssistantMessage { text, .. },
        ProviderOutput::TurnCompleted { outcome },
    ] = outputs.as_slice()
    else {
        panic!("unexpected outputs: {outputs:#?}");
    };
    assert_eq!(session_id, "efe578f3-875d-4ecb-9bd7-af911b691388");
    assert_eq!(cwd, r"C:\work");
    assert_eq!(name, "Read");
    assert!(input.contains("note.txt"));
    assert_eq!(tool_id, result_id);
    assert!(output.contains("hello from hoshi"));
    assert!(!is_error);
    assert_eq!(text, "The file says: \"hello from hoshi\".");
    assert_eq!(*outcome, TurnOutcome::Completed);
}

#[test]
fn streamed_deltas_add_up_to_the_final_message_with_the_same_id() {
    let outputs = parse(READ_TURN);
    let Some((message_id, final_text)) = outputs.iter().find_map(|output| match output {
        ProviderOutput::AssistantMessage { item_id, text } => Some((item_id, text)),
        _ => None,
    }) else {
        panic!("no assistant message");
    };

    let streamed: String = outputs
        .iter()
        .filter_map(|output| match output {
            ProviderOutput::TextDelta { item_id, delta } if item_id == message_id => {
                Some(delta.as_str())
            }
            _ => None,
        })
        .collect();

    assert_eq!(&streamed, final_text);
}

#[test]
fn reasoning_streams_and_then_arrives_whole_before_the_reply() {
    let outputs = parse(THINKING_TURN);
    let Some((reasoning_id, reasoning)) = outputs.iter().find_map(|output| match output {
        ProviderOutput::Reasoning { item_id, text } => Some((item_id, text)),
        _ => None,
    }) else {
        panic!("no reasoning: {outputs:#?}");
    };

    let streamed: String = outputs
        .iter()
        .filter_map(|output| match output {
            ProviderOutput::ReasoningDelta { item_id, delta } if item_id == reasoning_id => {
                Some(delta.as_str())
            }
            _ => None,
        })
        .collect();
    let reasoning_at = outputs
        .iter()
        .position(|output| matches!(output, ProviderOutput::Reasoning { .. }));
    let reply_at = outputs
        .iter()
        .position(|output| matches!(output, ProviderOutput::AssistantMessage { .. }));

    assert_eq!(&streamed, reasoning);
    assert!(reasoning_at < reply_at, "{outputs:#?}");
    assert!(!outputs.iter().any(|output| matches!(
        output,
        ProviderOutput::TextDelta { item_id, .. } if item_id == reasoning_id
    )));
}

#[test]
fn an_error_result_fails_the_turn() {
    let mut parser = ClaudeParser::default();

    let outputs = parser.parse_line(
        r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"boom"}"#,
    );

    assert!(matches!(
        outputs.as_slice(),
        [ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Failed { .. }
        }]
    ));
}

#[test]
fn subagent_traffic_goes_to_its_own_transcript() {
    let mut parser = ClaudeParser::default();

    let message = parser.parse_line(
        r#"{"type":"assistant","parent_tool_use_id":"toolu_1","message":{"id":"m","content":[{"type":"text","text":"inner"}]}}"#,
    );
    let prompt = parser.parse_line(
        r#"{"type":"user","parent_tool_use_id":"toolu_1","message":{"role":"user","content":[{"type":"text","text":"count the files"}]}}"#,
    );
    let streamed = parser.parse_line(
        r#"{"type":"stream_event","parent_tool_use_id":"toolu_1","api_message_id":"m","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"in"}}}"#,
    );
    let progress = parser.parse_line(
        r#"{"type":"system","subtype":"task_progress","task_id":"a1","tool_use_id":"toolu_1","description":"Running ls"}"#,
    );

    assert_eq!(
        message,
        vec![ProviderOutput::Subagent {
            tool_use_id: "toolu_1".into(),
            output: Box::new(ProviderOutput::AssistantMessage {
                item_id: "m:0".into(),
                text: "inner".into(),
            }),
        }]
    );
    assert!(prompt.is_empty());
    assert!(streamed.is_empty());
    assert_eq!(
        progress,
        vec![ProviderOutput::ToolOutput {
            item_id: "toolu_1".into(),
            output: "Running ls".into(),
        }]
    );
}

const USAGE_TURN: &str = concat!(
    r#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"claude-haiku-5-5","id":"msg_1","usage":{"input_tokens":2,"cache_creation_input_tokens":5096,"cache_read_input_tokens":3969,"output_tokens":4}}},"parent_tool_use_id":null,"api_message_id":"msg_1"}"#,
    "
",
    r#"{"type":"stream_event","event":{"type":"message_start","message":{"model":"claude-sonnet-5-5","id":"msg_sub","usage":{"input_tokens":90000,"output_tokens":1}}},"parent_tool_use_id":"toolu_1","api_message_id":"msg_sub"}"#,
    "
",
    r#"{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"input_tokens":2,"cache_creation_input_tokens":5096,"cache_read_input_tokens":3969,"output_tokens":40}},"parent_tool_use_id":null,"api_message_id":"msg_1"}"#,
    "
",
    r#"{"type":"result","subtype":"success","is_error":false,"modelUsage":{"claude-haiku-5-5":{"contextWindow":1000000},"claude-sonnet-5-5":{"contextWindow":200000}}}"#,
);

fn context_usages(parser: &mut ClaudeParser, fixture: &str) -> Vec<ContextUsage> {
    fixture
        .lines()
        .flat_map(|line| parser.parse_line(line))
        .filter_map(|output| match output {
            ProviderOutput::ContextUsage(usage) => Some(usage),
            _ => None,
        })
        .collect()
}

#[test]
fn context_usage_counts_cached_input_and_output_of_the_agent_only() {
    let usages = context_usages(&mut ClaudeParser::new(None), USAGE_TURN);

    assert_eq!(
        usages,
        [
            ContextUsage {
                used_tokens: 9_071,
                max_tokens: None,
                ..ContextUsage::default()
            },
            ContextUsage {
                used_tokens: 9_107,
                max_tokens: None,
                ..ContextUsage::default()
            },
            ContextUsage {
                used_tokens: 9_107,
                max_tokens: Some(1_000_000),
                ..ContextUsage::default()
            },
        ]
    );
}

#[test]
fn a_window_known_up_front_is_not_reported_again() {
    let usages = context_usages(&mut ClaudeParser::new(Some(1_000_000)), USAGE_TURN);

    assert_eq!(usages.len(), 2);
    assert!(
        usages
            .iter()
            .all(|usage| usage.max_tokens == Some(1_000_000))
    );
}

#[test]
fn a_rate_limit_event_reports_the_plan_limits() {
    let mut parser = ClaudeParser::default();

    let outputs = parser.parse_line(
        r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","unifiedWindows":{"five_hour":{"utilization":0.34,"resetsAt":1794099000},"seven_day":{"utilization":0.16,"resetsAt":1794492000}}}}"#,
    );

    let [ProviderOutput::UsageLimits(limits)] = outputs.as_slice() else {
        panic!("unexpected outputs: {outputs:#?}");
    };
    assert_eq!(limits.windows[0].kind, UsageWindowKind::Session);
    assert_eq!(limits.windows[0].used_percent, 34);
    assert_eq!(limits.windows[1].kind, UsageWindowKind::Weekly);
}

/// Lines as Claude Code 2.1.319 prints them when the agent enters an existing worktree and
/// leaves it again; paths shortened.
const ENTER_WORKTREE: [&str; 2] = [
    r#"{"type":"assistant","message":{"id":"msg_1","content":[{"type":"tool_use","id":"toolu_enter","name":"EnterWorktree","input":{"path":"C:/repo-feature"}}]},"parent_tool_use_id":null}"#,
    r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_enter","type":"tool_result","content":"Entered worktree at “C:\\repo-feature” on branch “feat/existing”."}]},"parent_tool_use_id":null,"tool_use_result":{"worktreePath":"C:\\repo-feature","worktreeBranch":"feat/existing","message":"Entered worktree"}}"#,
];
const EXIT_WORKTREE: [&str; 2] = [
    r#"{"type":"assistant","message":{"id":"msg_2","content":[{"type":"tool_use","id":"toolu_exit","name":"ExitWorktree","input":{"action":"keep"}}]},"parent_tool_use_id":null}"#,
    r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_exit","type":"tool_result","content":"Exited worktree."}]},"parent_tool_use_id":null,"tool_use_result":{"action":"keep","originalCwd":"C:\\repo","worktreePath":"C:\\repo-feature","restoredCwd":"C:\\repo","message":"Exited worktree."}}"#,
];

fn working_directories(lines: &[&str]) -> Vec<String> {
    let mut parser = ClaudeParser::default();
    lines
        .iter()
        .flat_map(|line| parser.parse_line(line))
        .filter_map(|output| match output {
            ProviderOutput::WorkingDirectory { path } => Some(path),
            _ => None,
        })
        .collect()
}

#[test]
fn entering_and_leaving_a_worktree_report_where_the_session_works() {
    let lines: Vec<&str> = ENTER_WORKTREE
        .iter()
        .chain(&EXIT_WORKTREE)
        .copied()
        .collect();

    assert_eq!(
        working_directories(&lines),
        [r"C:\repo-feature", r"C:\repo"]
    );
}

/// Claude Code starts every run of a process with an `init` naming the folder it works in,
/// as when a background command's end wakes the agent within the same turn.
fn init(cwd: &str) -> String {
    serde_json::json!({
        "type": "system",
        "subtype": "init",
        "session_id": "s1",
        "cwd": cwd,
    })
    .to_string()
}

#[test]
fn later_runs_of_a_process_do_not_report_its_folder_again() {
    let first = init(r"C:\repo");
    let again = init(r"C:\repo");
    let resumed_elsewhere = init(r"C:\repo-feature");

    assert_eq!(working_directories(&[&first, &again]), [r"C:\repo"]);
    assert_eq!(
        working_directories(&[
            &first,
            ENTER_WORKTREE[0],
            ENTER_WORKTREE[1],
            &resumed_elsewhere
        ]),
        [r"C:\repo", r"C:\repo-feature"]
    );
    assert_eq!(
        working_directories(&[&first, &resumed_elsewhere]),
        [r"C:\repo", r"C:\repo-feature"]
    );
}

#[test]
fn a_refused_move_or_another_tool_with_a_similar_result_moves_nothing() {
    let refused = ENTER_WORKTREE[1].replace(
        r#""type":"tool_result","#,
        r#""type":"tool_result","is_error":true,"#,
    );
    let other_tool = ENTER_WORKTREE[0].replace("EnterWorktree", "Bash");

    assert!(working_directories(&[ENTER_WORKTREE[0], &refused]).is_empty());
    assert!(working_directories(&[&other_tool, ENTER_WORKTREE[1]]).is_empty());
    assert!(working_directories(&[ENTER_WORKTREE[1]]).is_empty());
}
