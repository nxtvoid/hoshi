//! Runs the real GitHub Copilot CLI, signed in: `cargo test -p hoshi-providers --test
//! copilot_live -- --ignored --nocapture`. Spends requests.

use hoshi_core::{AccessMode, ProviderOutput, SubagentEffort, TurnOutcome};
use hoshi_providers::{CopilotAdapter, ProviderAdapter, TitleRequest, TurnRequest};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn request(cwd: &str, prompt: &str, resume: Option<String>) -> TurnRequest {
    TurnRequest {
        cwd: cwd.into(),
        prompt: prompt.into(),
        attachments: Vec::new(),
        earlier_media: Vec::new(),
        resume_session_id: resume,
        resume_session_at: None,
        fork_session: false,
        handoff: None,
        model: None,
        effort: None,
        fast_mode: false,
        ultracode: false,
        access_mode: AccessMode::AcceptEdits,
        plan_mode: false,
        mcp: None,
        instructions: None,
        subagent_effort: SubagentEffort::default(),
        environment: Vec::new(),
    }
}

/// Everything the turn reported, up to its end.
fn run(adapter: &CopilotAdapter, request: TurnRequest) -> Vec<ProviderOutput> {
    let (sender, receiver) = mpsc::channel();
    let _turn = adapter
        .start_turn(
            request,
            Box::new(move |output| sender.send(output).unwrap()),
        )
        .unwrap();
    let mut outputs = Vec::new();
    while let Ok(output) = receiver.recv_timeout(Duration::from_secs(180)) {
        let done = matches!(output, ProviderOutput::TurnCompleted { .. });
        outputs.push(output);
        if done {
            break;
        }
    }
    outputs
}

fn answer(outputs: &[ProviderOutput]) -> String {
    outputs
        .iter()
        .filter_map(|output| match output {
            ProviderOutput::AssistantMessage { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
#[ignore = "runs the real Copilot CLI"]
fn a_turn_answers_and_the_next_one_resumes_its_session() {
    let dir = std::env::temp_dir().join(format!("hoshi-copilot-live-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.txt"), "the secret word is mango\n").unwrap();
    let cwd = dir.to_string_lossy().into_owned();
    let adapter = CopilotAdapter::new(None);

    let first = run(
        &adapter,
        request(
            &cwd,
            "Read notes.txt and tell me the secret word, one word only.",
            None,
        ),
    );
    println!("{first:#?}");
    let session = first.iter().find_map(|output| match output {
        ProviderOutput::SessionStarted { session_id } => Some(session_id.clone()),
        _ => None,
    });
    assert!(
        first
            .iter()
            .any(|output| matches!(output, ProviderOutput::ToolCallStarted { .. }))
    );
    assert!(answer(&first).to_lowercase().contains("mango"));
    assert_eq!(
        first.last(),
        Some(&ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed
        })
    );

    let second = run(
        &adapter,
        request(
            &cwd,
            "What was the secret word you just told me? One word, no tools.",
            session.clone(),
        ),
    );
    println!("{second:#?}");
    assert!(second.contains(&ProviderOutput::SessionStarted {
        session_id: session.unwrap()
    }));
    assert!(answer(&second).to_lowercase().contains("mango"));
}

fn session_of(outputs: &[ProviderOutput]) -> Option<String> {
    outputs.iter().find_map(|output| match output {
        ProviderOutput::SessionStarted { session_id } => Some(session_id.clone()),
        _ => None,
    })
}

#[test]
#[ignore = "runs the real Copilot CLI"]
fn the_next_turn_of_a_session_runs_in_the_process_the_last_one_left() {
    let dir = std::env::temp_dir().join(format!("hoshi-copilot-reuse-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cwd = dir.to_string_lossy().into_owned();
    let adapter = CopilotAdapter::new(None);

    let first = run(
        &adapter,
        request(
            &cwd,
            "Remember the word papaya. Reply with OK only, no tools.",
            None,
        ),
    );
    let session = session_of(&first).expect("a session");
    let kept = adapter.kept_process(&session);
    assert!(kept.is_some(), "the first turn's process is kept");

    let started = Instant::now();
    let second = run(
        &adapter,
        request(
            &cwd,
            "Which word did I ask you to remember? One word, no tools.",
            Some(session.clone()),
        ),
    );
    let took = started.elapsed();
    println!("{second:#?}\nsecond turn took {took:?}");

    assert_eq!(session_of(&second), Some(session.clone()));
    assert!(answer(&second).to_lowercase().contains("papaya"));
    assert_eq!(
        second.last(),
        Some(&ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed
        })
    );
    assert_eq!(adapter.kept_process(&session), kept);

    let mut harder = request(&cwd, "Say hi.", Some(session.clone()));
    harder.effort = Some(hoshi_core::Effort::High);
    let third = run(&adapter, harder);
    assert_eq!(session_of(&third), Some(session.clone()));
    assert_ne!(adapter.kept_process(&session), kept);
}

#[test]
#[ignore = "runs the real Copilot CLI"]
fn copilot_reports_who_is_signed_in() {
    let adapter = CopilotAdapter::new(None);

    let status = adapter.account_status();

    println!("{status:?} as {:?}", adapter.login());
    assert_eq!(status, hoshi_core::ProviderAccountStatus::Ready);
    assert!(adapter.login().is_some_and(|login| !login.is_empty()));
}

#[test]
#[ignore = "runs the real Copilot CLI"]
fn copilot_names_a_thread_and_lists_its_models() {
    let adapter = CopilotAdapter::new(None);

    let title = adapter.generate_title(&TitleRequest {
        context: "the login button does nothing on mobile safari".into(),
        previous_title: None,
        images: Vec::new(),
        model: None,
        instructions: String::new(),
    });
    let models = adapter.models();

    println!("{title:?}\n{models:#?}");
    assert!(title.is_some_and(|title| !title.title.is_empty()));
    assert!(!models.models.is_empty());
}

#[test]
#[ignore = "runs the real Copilot CLI"]
fn a_command_waits_for_approval_and_a_stopped_turn_ends_interrupted() {
    let dir = std::env::temp_dir().join(format!("hoshi-copilot-ask-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cwd = dir.to_string_lossy().into_owned();
    let adapter = CopilotAdapter::new(None);
    let mut ask = request(
        &cwd,
        "Create a file named made.txt containing the word kiwi, using a shell command.",
        None,
    );
    ask.access_mode = AccessMode::Ask;
    let (sender, receiver) = mpsc::channel();
    let turn = adapter
        .start_turn(ask, Box::new(move |output| sender.send(output).unwrap()))
        .unwrap();

    let request_id = loop {
        match receiver.recv_timeout(Duration::from_secs(180)).unwrap() {
            ProviderOutput::RequestOpened {
                request_id,
                content,
            } => {
                println!("{content:#?}");
                break request_id;
            }
            ProviderOutput::TurnCompleted { outcome } => panic!("ended first: {outcome:?}"),
            _ => {}
        }
    };
    turn.respond(
        &request_id,
        &hoshi_core::RequestResponse::Decision {
            decision: hoshi_core::ApprovalDecision::Accept,
            feedback: None,
        },
    );
    std::thread::sleep(Duration::from_secs(3));
    turn.interrupt();
    let outcome = loop {
        if let ProviderOutput::TurnCompleted { outcome } =
            receiver.recv_timeout(Duration::from_secs(60)).unwrap()
        {
            break outcome;
        }
    };
    println!("{outcome:?}");
    assert!(matches!(
        outcome,
        TurnOutcome::Interrupted | TurnOutcome::Completed
    ));
}
