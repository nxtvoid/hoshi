use hoshi_core::{ProviderOutput, TurnOutcome};
use hoshi_providers::ClaudeParser;

const READ_TURN: &str = include_str!("fixtures/claude-read-turn.jsonl");

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
        .filter(|output| !matches!(output, ProviderOutput::TextDelta { .. }))
        .collect();

    let [
        ProviderOutput::SessionStarted { session_id },
        ProviderOutput::ToolCallStarted {
            item_id: tool_id,
            name,
            input,
        },
        ProviderOutput::ToolCallCompleted {
            item_id: result_id,
            output,
            is_error,
        },
        ProviderOutput::AssistantMessage { text, .. },
        ProviderOutput::TurnCompleted { outcome },
    ] = outputs.as_slice()
    else {
        panic!("unexpected outputs: {outputs:#?}");
    };
    assert_eq!(session_id, "efe578f3-875d-4ecb-9bd7-af911b691388");
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
fn subagent_traffic_is_ignored() {
    let mut parser = ClaudeParser::default();

    let outputs = parser.parse_line(
        r#"{"type":"assistant","parent_tool_use_id":"toolu_1","message":{"id":"m","content":[{"type":"text","text":"inner"}]}}"#,
    );

    assert!(outputs.is_empty());
}
