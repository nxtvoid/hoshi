//! Runs a real Claude Code turn. Costs a few cents, so it only runs on request:
//! `cargo test -p hoshi-providers --test claude_live -- --ignored`

use hoshi_core::{ProviderOutput, TurnOutcome};
use hoshi_providers::{ClaudeCodeAdapter, ProviderAdapter, TurnRequest};
use std::sync::mpsc;

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn a_real_turn_streams_a_reply_and_resumes_its_session() {
    let adapter = ClaudeCodeAdapter {
        binary_path: None,
        model: Some("haiku".into()),
        permission_mode: "acceptEdits".into(),
    };
    let cwd = std::env::temp_dir().to_string_lossy().into_owned();
    let run = |prompt: &str, resume: Option<String>| {
        let (sender, receiver) = mpsc::channel();
        let request = TurnRequest {
            cwd: cwd.clone(),
            prompt: prompt.into(),
            resume_session_id: resume,
            model: None,
            effort: Some(hoshi_core::Effort::Low),
        };
        let sink = Box::new(move |output| {
            let _ = sender.send(output);
        });
        let _turn = adapter.start_turn(request, sink).unwrap();
        receiver.iter().collect::<Vec<_>>()
    };

    let first = run("Reply with exactly the word: apple", None);
    let session_id = first.iter().find_map(|output| match output {
        ProviderOutput::SessionStarted { session_id } => Some(session_id.clone()),
        _ => None,
    });
    let second = run(
        "What word did you just reply with? Answer with one word.",
        session_id,
    );

    for outputs in [&first, &second] {
        assert!(
            outputs
                .iter()
                .any(|o| matches!(o, ProviderOutput::TextDelta { .. }))
        );
        assert!(matches!(
            outputs.last(),
            Some(ProviderOutput::TurnCompleted {
                outcome: TurnOutcome::Completed
            })
        ));
    }
    let reply = second.iter().find_map(|output| match output {
        ProviderOutput::AssistantMessage { text, .. } => Some(text.to_lowercase()),
        _ => None,
    });
    assert!(
        reply.is_some_and(|text| text.contains("apple")),
        "{second:#?}"
    );
}
