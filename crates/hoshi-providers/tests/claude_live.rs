//! Runs a real Claude Code turn. Costs a few cents, so it only runs on request:
//! `cargo test -p hoshi-providers --test claude_live -- --ignored`

use hoshi_core::{
    AccessMode, ApprovalDecision, ItemContent, ProviderOutput, RequestResponse, ToolDetail,
    TurnOutcome,
};
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

/// Runs one turn in `cwd`, answering each request it opens with `answer`.
fn run_answering(
    cwd: &std::path::Path,
    prompt: &str,
    access_mode: AccessMode,
    mut answer: impl FnMut(&ItemContent) -> RequestResponse,
) -> Vec<ProviderOutput> {
    let adapter = ClaudeCodeAdapter::new(None, Some("haiku".into()));
    let (sender, receiver) = mpsc::channel();
    let request = TurnRequest {
        cwd: cwd.to_string_lossy().into_owned(),
        prompt: prompt.into(),
        attachments: Vec::new(),
        resume_session_id: None,
        model: None,
        effort: Some(hoshi_core::Effort::Low),
        fast_mode: false,
        ultracode: false,
        access_mode,
        plan_mode: false,
    };
    let sink = Box::new(move |output| {
        let _ = sender.send(output);
    });
    let turn = adapter.start_turn(request, sink).unwrap();
    let mut outputs = Vec::new();
    for output in receiver.iter() {
        if let ProviderOutput::RequestOpened {
            request_id,
            content,
        } = &output
        {
            turn.respond(request_id, &answer(content));
        }
        outputs.push(output);
    }
    outputs
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("hoshi-live-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn an_approved_tool_call_runs_mid_turn() {
    let cwd = scratch("approval");
    let outputs = run_answering(
        &cwd,
        "Create a folder named approved-folder in the current directory with a shell command, then reply done.",
        AccessMode::Ask,
        |content| {
            assert!(
                matches!(content, ItemContent::Approval { .. }),
                "{content:?}"
            );
            RequestResponse::Decision {
                decision: ApprovalDecision::Accept,
                feedback: None,
            }
        },
    );

    assert!(
        outputs
            .iter()
            .any(|output| matches!(output, ProviderOutput::RequestOpened { .. })),
        "{outputs:#?}"
    );
    assert!(cwd.join("approved-folder").is_dir(), "{outputs:#?}");
    assert!(matches!(
        outputs.last(),
        Some(ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed
        })
    ));
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn an_answered_question_reaches_the_agent() {
    let cwd = scratch("question");
    let outputs = run_answering(
        &cwd,
        "Use the AskUserQuestion tool to ask me whether I prefer red or blue, then reply with my answer in one word.",
        AccessMode::AcceptEdits,
        |content| {
            let ItemContent::Questions { questions, .. } = content else {
                panic!("expected questions, got {content:?}");
            };
            RequestResponse::Answers {
                answers: questions.iter().map(|_| "Blue".to_owned()).collect(),
            }
        },
    );

    let reply = outputs.iter().rev().find_map(|output| match output {
        ProviderOutput::AssistantMessage { text, .. } => Some(text.to_lowercase()),
        _ => None,
    });
    assert!(
        reply.is_some_and(|text| text.contains("blue")),
        "{outputs:#?}"
    );
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn an_edit_reports_its_diff() {
    let cwd = scratch("diff");
    let outputs = run_answering(
        &cwd,
        "Use the Write tool to create notes.txt with the lines one, two, three. Then use the Edit tool to change two to TWO. Reply done.",
        AccessMode::AcceptEdits,
        |_| RequestResponse::Decision {
            decision: ApprovalDecision::Accept,
            feedback: None,
        },
    );

    let changes: Vec<_> = outputs
        .iter()
        .filter_map(|output| match output {
            ProviderOutput::ToolCallCompleted {
                detail:
                    Some(ToolDetail::FileChange {
                        additions,
                        deletions,
                        diff,
                        ..
                    }),
                ..
            } => Some((*additions, *deletions, diff.clone().unwrap_or_default())),
            _ => None,
        })
        .collect();
    assert!(
        changes
            .iter()
            .any(|(added, removed, diff)| *added == 1 && *removed == 1 && diff.contains("+TWO")),
        "{outputs:#?}"
    );
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn the_task_list_is_reported_as_it_changes() {
    let cwd = scratch("tasks");
    let outputs = run_answering(
        &cwd,
        "Use your task list tools to add two tasks, 'first' and 'second', then mark 'first' completed. Do nothing else and reply done.",
        AccessMode::AcceptEdits,
        |_| RequestResponse::Decision {
            decision: ApprovalDecision::Accept,
            feedback: None,
        },
    );

    let last = outputs.iter().rev().find_map(|output| match output {
        ProviderOutput::TaskList { tasks } => Some(tasks.clone()),
        _ => None,
    });
    let tasks = last.unwrap_or_else(|| panic!("no task list in {outputs:#?}"));
    assert_eq!(tasks.len(), 2, "{tasks:#?}");
    assert_eq!(tasks[0].status, hoshi_core::TaskStatus::Completed);
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn a_running_command_shows_its_output_before_it_ends() {
    let cwd = scratch("live-output");
    let command = if cfg!(windows) {
        "1..6 | % { \"tick $_\"; Start-Sleep -Milliseconds 700 }"
    } else {
        "for i in 1 2 3 4 5 6; do echo tick $i; sleep 0.7; done"
    };
    let outputs = run_answering(
        &cwd,
        &format!("Run this exact shell command in the foreground and then reply done: {command}"),
        AccessMode::FullAccess,
        |_| RequestResponse::Decision {
            decision: ApprovalDecision::Accept,
            feedback: None,
        },
    );

    let completed = outputs
        .iter()
        .position(|output| matches!(output, ProviderOutput::ToolCallCompleted { output, .. } if output.contains("tick 6")))
        .unwrap_or_else(|| panic!("the command never completed: {outputs:#?}"));
    let partial = outputs[..completed].iter().any(|output| {
        matches!(output, ProviderOutput::ToolOutput { output, .. }
            if output.contains("tick 1") && !output.contains("tick 6"))
    });
    assert!(partial, "{outputs:#?}");
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn a_subagent_reports_its_own_work() {
    let cwd = scratch("subagent");
    let outputs = run_answering(
        &cwd,
        "Use the Agent tool to launch one general-purpose subagent in the foreground that lists the files in the current directory with a shell command. Then reply with its answer.",
        AccessMode::FullAccess,
        |_| RequestResponse::Decision {
            decision: ApprovalDecision::Accept,
            feedback: None,
        },
    );

    let agent = outputs.iter().find_map(|output| match output {
        ProviderOutput::ToolCallStarted { item_id, name, .. } if name == "Agent" => {
            Some(item_id.clone())
        }
        _ => None,
    });
    let agent = agent.unwrap_or_else(|| panic!("no subagent started: {outputs:#?}"));
    let inner = outputs.iter().any(|output| {
        matches!(output, ProviderOutput::Subagent { tool_use_id, output }
            if *tool_use_id == agent
                && matches!(**output, ProviderOutput::ToolCallStarted { .. }))
    });
    assert!(inner, "{outputs:#?}");
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn a_stopped_subagent_ends_and_the_turn_goes_on() {
    let cwd = scratch("stop-subagent");
    let adapter = ClaudeCodeAdapter::new(None, Some("haiku".into()));
    let (sender, receiver) = mpsc::channel();
    let request = TurnRequest {
        cwd: cwd.to_string_lossy().into_owned(),
        prompt: "Use the Agent tool to launch one general-purpose subagent in the foreground whose only job is to run this exact shell command in the foreground: `1..40 | % { Start-Sleep 1; \"tick $_\" }` (or `for i in $(seq 40); do sleep 1; echo tick $i; done`) and then report done. When it returns, reply with one word.".into(),
        attachments: Vec::new(),
        resume_session_id: None,
        model: None,
        effort: Some(hoshi_core::Effort::Low),
        fast_mode: false,
        ultracode: false,
        access_mode: AccessMode::FullAccess,
        plan_mode: false,
    };
    let sink = Box::new(move |output| {
        let _ = sender.send(output);
    });
    let turn = adapter.start_turn(request, sink).unwrap();
    let started = std::time::Instant::now();
    let mut agent: Option<String> = None;
    let mut stopped = false;
    let mut outputs = Vec::new();
    loop {
        let Ok(output) = receiver.recv_timeout(std::time::Duration::from_millis(200)) else {
            if let (Some(agent), false) = (&agent, stopped)
                && started.elapsed() > std::time::Duration::from_secs(12)
            {
                turn.stop_subagent(agent);
                stopped = true;
            }
            continue;
        };
        if let ProviderOutput::ToolCallStarted { item_id, name, .. } = &output
            && name == "Agent"
        {
            agent = Some(item_id.clone());
        }
        let done = matches!(output, ProviderOutput::TurnCompleted { .. });
        outputs.push(output);
        if done {
            break;
        }
    }

    assert!(stopped, "the subagent never started: {outputs:#?}");
    let agent = agent.unwrap_or_default();
    assert!(
        outputs.iter().any(
            |output| matches!(output, ProviderOutput::ToolCallStopped { item_id } if *item_id == agent)
        ),
        "the subagent's call was not marked stopped: {outputs:#?}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(40),
        "the turn waited for the subagent: {outputs:#?}"
    );
    assert!(matches!(
        outputs.last(),
        Some(ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed
        })
    ));
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn a_turn_waits_for_a_background_subagent() {
    let cwd = scratch("background-agent");
    let outputs = run_answering(
        &cwd,
        "Launch one general-purpose subagent with the Agent tool IN THE BACKGROUND (run_in_background: true) whose job is to run the shell command `1..5 | % { Start-Sleep 1; \"tick $_\" }` (or `for i in 1 2 3 4 5; do sleep 1; echo tick $i; done`) and report the last line. Then reply 'launched' and end your turn.",
        AccessMode::FullAccess,
        |_| RequestResponse::Decision {
            decision: ApprovalDecision::Accept,
            feedback: None,
        },
    );

    let agent = outputs.iter().find_map(|output| match output {
        ProviderOutput::ToolCallStarted { item_id, name, .. } if name == "Agent" => {
            Some(item_id.clone())
        }
        _ => None,
    });
    let agent = agent.unwrap_or_else(|| panic!("no subagent: {outputs:#?}"));
    let closed = outputs.iter().position(|output| {
        matches!(output, ProviderOutput::ToolCallCompleted { item_id, output, .. }
            if *item_id == agent && !output.contains("Async agent launched"))
    });
    let closed = closed.unwrap_or_else(|| panic!("the subagent's call never closed: {outputs:#?}"));
    let completions = outputs
        .iter()
        .filter(|output| matches!(output, ProviderOutput::TurnCompleted { .. }))
        .count();
    assert_eq!(completions, 1, "{outputs:#?}");
    let answered_after = outputs[closed..]
        .iter()
        .any(|output| matches!(output, ProviderOutput::AssistantMessage { .. }));
    assert!(answered_after, "{outputs:#?}");
}
