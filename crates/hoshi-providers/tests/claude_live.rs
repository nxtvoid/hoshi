//! Runs a real Claude Code turn. Costs a few cents, so it only runs on request:
//! `cargo test -p hoshi-providers --test claude_live -- --ignored`

use hoshi_core::{ProviderOutput, TurnOutcome};
use hoshi_providers::{ClaudeCodeAdapter, ProviderAdapter, TurnRequest};
use std::sync::mpsc;

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn a_real_turn_streams_a_reply_and_resumes_its_session() {
    let adapter = ClaudeCodeAdapter::new(None, Some("haiku".into()));
    let cwd = std::env::temp_dir().to_string_lossy().into_owned();
    let run = |prompt: &str, resume: Option<String>| {
        let (sender, receiver) = mpsc::channel();
        let request = TurnRequest {
            cwd: cwd.clone(),
            prompt: prompt.into(),
            attachments: Vec::new(),
            resume_session_id: resume,
            model: None,
            effort: Some(hoshi_core::Effort::Low),
            fast_mode: false,
            ultracode: false,
            access_mode: hoshi_core::AccessMode::AcceptEdits,
            plan_mode: false,
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

#[test]
#[ignore = "runs the real Claude Code CLI"]
fn the_cli_reports_the_plan_limits() {
    let adapter = ClaudeCodeAdapter::new(None, None);

    let limits = adapter.usage_limits().expect("the CLI answers get_usage");

    println!("{limits:#?}");
}

#[test]
#[ignore = "runs the real Claude Code CLI and looks up its latest release"]
fn the_cli_reports_its_installation() {
    let adapter = ClaudeCodeAdapter::new(None, None);

    let installation = adapter.installation().expect("the adapter knows its CLI");

    println!("{installation:#?}");
    assert!(installation.installed.is_some());
    assert!(installation.latest.is_some());
}

#[test]
#[ignore = "runs the real Claude Code CLI"]
fn the_cli_lists_its_slash_commands() {
    let adapter = ClaudeCodeAdapter::new(None, None);

    let commands = adapter
        .slash_commands(&std::env::temp_dir())
        .expect("the CLI answers initialize");

    assert!(commands.iter().any(|command| command.name == "compact"));
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn a_real_turn_sees_an_attached_image() {
    use base64::Engine as _;
    let adapter = ClaudeCodeAdapter::new(None, Some("haiku".into()));
    let dir = std::env::temp_dir().join("hoshi-live-image");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("red.png");
    // An 8x8 red PNG.
    let png = base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAgAAAAICAIAAABLbSncAAAAEklEQVR4nGP4z8CAFWEXHbQSACj/P8Fu7N9hAAAAAElFTkSuQmCC")
        .unwrap();
    std::fs::write(&path, png).unwrap();
    let (sender, receiver) = mpsc::channel();
    let request = TurnRequest {
        cwd: dir.to_string_lossy().into_owned(),
        prompt: "What single color fills this image? Reply with one lowercase word.".into(),
        attachments: vec![hoshi_providers::TurnAttachment {
            attachment: hoshi_core::MessageAttachment::Image {
                id: "red".into(),
                name: "red.png".into(),
                mime_type: "image/png".into(),
                size: 0,
            },
            path: Some(path),
        }],
        resume_session_id: None,
        model: None,
        effort: Some(hoshi_core::Effort::Low),
        fast_mode: false,
        ultracode: false,
        access_mode: hoshi_core::AccessMode::AcceptEdits,
        plan_mode: false,
    };
    let _turn = adapter
        .start_turn(
            request,
            Box::new(move |output| {
                let _ = sender.send(output);
            }),
        )
        .unwrap();
    let outputs: Vec<_> = receiver.iter().collect();

    let reply = outputs.iter().find_map(|output| match output {
        ProviderOutput::AssistantMessage { text, .. } => Some(text.to_lowercase()),
        _ => None,
    });
    assert!(
        reply.is_some_and(|text| text.contains("red")),
        "{outputs:#?}"
    );
}

#[test]
#[ignore = "spends tokens on a small Claude Code run"]
fn a_real_run_titles_a_thread() {
    let adapter = ClaudeCodeAdapter::new(None, None);
    let request = hoshi_providers::TitleRequest {
        context: "the login page redirects to /home even when the session expired, fix it".into(),
        previous_title: None,
        images: Vec::new(),
        model: None,
        instructions: String::new(),
    };

    let title = adapter.generate_title(&request).expect("a title");
    println!("{title:?}");

    assert!(!title.title.is_empty() && title.title.chars().count() < 60);
}
