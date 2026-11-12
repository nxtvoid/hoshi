//! Runs a real Claude Code turn. Costs a few cents, so it only runs on request:
//! `cargo test -p hoshi-providers --test claude_live -- --ignored`

use hoshi_core::{
    AccessMode, ApprovalDecision, ItemContent, ProviderOutput, RequestResponse, SubagentEffort,
    ToolDetail, TurnOutcome,
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
            earlier_media: Vec::new(),
            resume_session_id: resume,
            resume_session_at: None,
            model: None,
            effort: Some(hoshi_core::Effort::Low),
            fast_mode: false,
            ultracode: false,
            access_mode: hoshi_core::AccessMode::AcceptEdits,
            plan_mode: false,
            mcp: None,
            instructions: None,
            subagent_effort: SubagentEffort::Thread,
            environment: Vec::new(),
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
            last_output(outputs),
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
#[ignore = "spends tokens on a real Claude Code turn"]
fn a_message_sent_as_the_turn_starts_doesnt_take_the_place_of_its_prompt() {
    let adapter = ClaudeCodeAdapter::new(None, Some("haiku".into()));
    let (sender, receiver) = mpsc::channel();
    let request = TurnRequest {
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        prompt: "The first word is ALPHA. Wait for the second word.".into(),
        attachments: Vec::new(),
        earlier_media: Vec::new(),
        resume_session_id: None,
        resume_session_at: None,
        model: None,
        effort: Some(hoshi_core::Effort::Low),
        fast_mode: false,
        ultracode: false,
        access_mode: hoshi_core::AccessMode::AcceptEdits,
        plan_mode: false,
        mcp: None,
        instructions: None,
        subagent_effort: SubagentEffort::Thread,
        environment: Vec::new(),
    };
    let sink = Box::new(move |output| {
        let _ = sender.send(output);
    });
    let turn = adapter.start_turn(request, sink).unwrap();

    let mut said = String::new();
    let mut sent = false;
    loop {
        match receiver.recv_timeout(std::time::Duration::from_secs(90)) {
            Ok(ProviderOutput::SessionStarted { .. }) if !sent => {
                sent = true;
                assert!(turn.steer(
                    "7c1e4a2b-3d5f-4a6b-8c9d-0e1f2a3b4c5d",
                    "The second word is BETA. Reply with both words I gave you, and nothing else.",
                    &[],
                    true
                ));
            }
            Ok(ProviderOutput::AssistantMessage { text, .. }) => said.push_str(&text),
            Ok(ProviderOutput::TurnCompleted { .. }) => break,
            Ok(_) => {}
            Err(_) => panic!("the turn never ended; it said: {said}"),
        }
    }
    assert!(said.contains("ALPHA") && said.contains("BETA"), "{said}");
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
            extracted_text: None,
        }],
        earlier_media: Vec::new(),
        resume_session_id: None,
        resume_session_at: None,
        model: None,
        effort: Some(hoshi_core::Effort::Low),
        fast_mode: false,
        ultracode: false,
        access_mode: hoshi_core::AccessMode::AcceptEdits,
        plan_mode: false,
        mcp: None,
        instructions: None,
        subagent_effort: SubagentEffort::Thread,
        environment: Vec::new(),
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

/// A one-page PDF that shows `text`, with a cross-reference table PDF readers accept.
fn pdf_showing(text: &str) -> Vec<u8> {
    let stream = format!("BT /F1 24 Tf 72 700 Td ({text}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_owned(),
        format!("<< /Length {} >>
stream
{stream}
endstream", stream.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let mut pdf = String::from(
        "%PDF-1.4
",
    );
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!(
            "{} 0 obj
{object}
endobj
",
            index + 1
        ));
    }
    let xref = pdf.len();
    pdf.push_str(&format!(
        "xref
0 {}
0000000000 65535 f 
",
        objects.len() + 1
    ));
    for offset in offsets {
        pdf.push_str(&format!(
            "{offset:010} 00000 n 
"
        ));
    }
    pdf.push_str(&format!(
        "trailer
<< /Size {} /Root 1 0 R >>
startxref
{xref}
%%EOF
",
        objects.len() + 1
    ));
    pdf.into_bytes()
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn a_real_turn_reads_an_attached_pdf_and_a_file_by_its_path() {
    let adapter = ClaudeCodeAdapter::new(None, Some("haiku".into()));
    let dir = std::env::temp_dir().join("hoshi-live-files");
    std::fs::create_dir_all(&dir).unwrap();
    let pdf = dir.join("memo.pdf");
    std::fs::write(&pdf, pdf_showing("The first code word is PELICAN.")).unwrap();
    let notes = dir.join("notes.txt");
    std::fs::write(
        &notes,
        "The second code word is HERON.
",
    )
    .unwrap();
    let file =
        |name: &str, mime_type: &str, path: std::path::PathBuf| hoshi_providers::TurnAttachment {
            attachment: hoshi_core::MessageAttachment::File {
                id: name.into(),
                name: name.into(),
                mime_type: mime_type.into(),
                size: 0,
            },
            path: Some(path),
            extracted_text: None,
        };
    let (sender, receiver) = mpsc::channel();
    let request = TurnRequest {
        cwd: dir.to_string_lossy().into_owned(),
        prompt: "[File #1] and [File #2] each hold one code word; read both files. Reply with just the two code words."
            .into(),
        attachments: vec![
            file("memo.pdf", "application/pdf", pdf),
            file("notes.txt", "text/plain", notes),
        ],
        earlier_media: Vec::new(),
        resume_session_id: None,
        resume_session_at: None,
        model: None,
        effort: Some(hoshi_core::Effort::Low),
        fast_mode: false,
        ultracode: false,
        access_mode: hoshi_core::AccessMode::AcceptEdits,
        plan_mode: false,
        mcp: None,
        instructions: None,
        subagent_effort: SubagentEffort::Thread,
        environment: Vec::new(),
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

    let reply: String = outputs
        .iter()
        .filter_map(|output| match output {
            ProviderOutput::AssistantMessage { text, .. } => Some(text.to_lowercase()),
            _ => None,
        })
        .collect();
    assert!(
        reply.contains("pelican") && reply.contains("heron"),
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
        earlier_media: Vec::new(),
        resume_session_id: None,
        resume_session_at: None,
        model: None,
        effort: Some(hoshi_core::Effort::Low),
        fast_mode: false,
        ultracode: false,
        access_mode,
        plan_mode: false,
        mcp: None,
        instructions: None,
        subagent_effort: SubagentEffort::Thread,
        environment: Vec::new(),
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
        last_output(&outputs),
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
        earlier_media: Vec::new(),
        resume_session_id: None,
        resume_session_at: None,
        model: None,
        effort: Some(hoshi_core::Effort::Low),
        fast_mode: false,
        ultracode: false,
        access_mode: AccessMode::FullAccess,
        plan_mode: false,
        mcp: None,
        instructions: None,
        subagent_effort: SubagentEffort::Thread,
        environment: Vec::new(),
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
        last_output(&outputs),
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

/// Starts a turn running a slow command, calls `act` on it after `after`, and collects
/// everything until it ends.
fn run_acting(
    name: &str,
    after: std::time::Duration,
    act: impl FnOnce(&dyn hoshi_providers::RunningTurn),
) -> (Vec<ProviderOutput>, std::time::Duration) {
    let cwd = scratch(name);
    let adapter = ClaudeCodeAdapter::new(None, Some("haiku".into()));
    let (sender, receiver) = mpsc::channel();
    let request = TurnRequest {
        cwd: cwd.to_string_lossy().into_owned(),
        prompt: "Run this exact shell command in the foreground: `1..12 | % { Start-Sleep 1; \"tick $_\" }` (or `for i in $(seq 12); do sleep 1; echo tick $i; done`). Then reply with the last line it printed.".into(),
        attachments: Vec::new(),
        earlier_media: Vec::new(),
        resume_session_id: None,
        resume_session_at: None,
        model: None,
        effort: Some(hoshi_core::Effort::Low),
        fast_mode: false,
        ultracode: false,
        access_mode: AccessMode::FullAccess,
        plan_mode: false,
        mcp: None,
        instructions: None,
        subagent_effort: SubagentEffort::Thread,
        environment: Vec::new(),
    };
    let sink = Box::new(move |output| {
        let _ = sender.send(output);
    });
    let turn = adapter.start_turn(request, sink).unwrap();
    let started = std::time::Instant::now();
    let mut act = Some(act);
    let mut acted_at = None;
    let mut outputs = Vec::new();
    loop {
        if let Ok(output) = receiver.recv_timeout(std::time::Duration::from_millis(100)) {
            let done = matches!(output, ProviderOutput::TurnCompleted { .. });
            outputs.push(output);
            if done {
                break;
            }
        }
        if started.elapsed() > after
            && let Some(act) = act.take()
        {
            act(turn.as_ref());
            acted_at = Some(std::time::Instant::now());
        }
    }
    let since_act = acted_at.map(|at| at.elapsed()).unwrap_or_default();
    (outputs, since_act)
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn a_message_sent_mid_turn_reaches_the_agent() {
    let (outputs, _) = run_acting("steer", std::time::Duration::from_secs(4), |turn| {
        assert!(turn.steer(
            "5b8f3c2e-1d4a-4e6b-9f70-2a1c3d4e5f60",
            "Also: end your reply with the word BANANA.",
            &[],
            true
        ));
    });

    let completions = outputs
        .iter()
        .filter(|output| matches!(output, ProviderOutput::TurnCompleted { .. }))
        .count();
    let banana = outputs.iter().any(|output| {
        matches!(output, ProviderOutput::AssistantMessage { text, .. } if text.contains("BANANA"))
    });
    assert_eq!(completions, 1, "{outputs:#?}");
    assert!(banana, "{outputs:#?}");
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn an_interrupted_turn_winds_down_on_its_own() {
    let (outputs, since_interrupt) =
        run_acting("interrupt", std::time::Duration::from_secs(5), |turn| {
            turn.interrupt()
        });

    assert!(matches!(
        last_output(&outputs),
        Some(ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Interrupted
        })
    ));
    assert!(
        since_interrupt < std::time::Duration::from_secs(4),
        "it took {since_interrupt:?}, so it was killed"
    );
    let stopped: Vec<_> = outputs
        .iter()
        .filter_map(|output| match output {
            ProviderOutput::ToolCallStopped { item_id } => Some(item_id),
            _ => None,
        })
        .collect();
    let failed_unstopped = outputs.iter().any(|output| {
        matches!(output, ProviderOutput::ToolCallCompleted { item_id, is_error: true, .. }
            if !stopped.contains(&item_id))
    });
    assert!(
        !failed_unstopped,
        "a call cut by the stop shows as failed: {outputs:#?}"
    );
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn a_message_is_answered_at_once_while_a_subagent_keeps_working() {
    let cwd = scratch("steer-subagent");
    let adapter = ClaudeCodeAdapter::new(None, Some("haiku".into()));
    let (sender, receiver) = mpsc::channel();
    let request = TurnRequest {
        cwd: cwd.to_string_lossy().into_owned(),
        prompt: "Use the Agent tool to launch one general-purpose subagent IN THE FOREGROUND whose job is to run `1..12 | % { Start-Sleep 1; \"tick $_\" }` (or `for i in $(seq 12); do sleep 1; echo tick $i; done`) and report the last line. Then tell me the last line.".into(),
        attachments: Vec::new(),
        earlier_media: Vec::new(),
        resume_session_id: None,
        resume_session_at: None,
        model: None,
        effort: Some(hoshi_core::Effort::Low),
        fast_mode: false,
        ultracode: false,
        access_mode: AccessMode::FullAccess,
        plan_mode: false,
        mcp: None,
        instructions: None,
        subagent_effort: SubagentEffort::Thread,
        environment: Vec::new(),
    };
    let sink = Box::new(move |output| {
        let _ = sender.send(output);
    });
    let turn = adapter.start_turn(request, sink).unwrap();
    let mut agent: Option<String> = None;
    let mut steered = false;
    let mut outputs = Vec::new();
    let started = std::time::Instant::now();
    loop {
        let Ok(output) = receiver.recv_timeout(std::time::Duration::from_millis(100)) else {
            if agent.is_some() && !steered && started.elapsed() > std::time::Duration::from_secs(7)
            {
                assert!(turn.steer(
                    "7c9e6679-7425-40de-944b-e07fc1f90ae7",
                    "Now just say HOLA.",
                    &[],
                    true
                ));
                steered = true;
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

    let agent = agent.unwrap_or_else(|| panic!("no subagent: {outputs:#?}"));
    let hola = outputs.iter().position(|output| {
        matches!(output, ProviderOutput::AssistantMessage { text, .. } if text.contains("HOLA"))
    });
    let agent_done = outputs.iter().position(|output| {
        matches!(output, ProviderOutput::ToolCallCompleted { item_id, output, .. }
            if *item_id == agent && !output.contains("Async agent launched"))
    });
    let hola = hola.unwrap_or_else(|| panic!("never said HOLA: {outputs:#?}"));
    let agent_done = agent_done.unwrap_or_else(|| panic!("the subagent never ended: {outputs:#?}"));
    assert!(
        hola < agent_done,
        "HOLA waited for the subagent: {outputs:#?}"
    );
    let completions = outputs
        .iter()
        .filter(|output| matches!(output, ProviderOutput::TurnCompleted { .. }))
        .count();
    assert_eq!(completions, 1, "{outputs:#?}");
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn a_turn_waits_for_claude_to_answer_every_background_subagent() {
    let cwd = scratch("two-background-agents");
    let outputs = run_answering(
        &cwd,
        "Launch TWO general-purpose subagents with the Agent tool in one message, both IN THE BACKGROUND (run_in_background: true). Each uses no tools and only replies with one word: the first replies ALPHA, the second replies BETA. Then reply 'launched' and end your turn. When both have answered, tell me both words.",
        AccessMode::FullAccess,
        |_| RequestResponse::Decision {
            decision: ApprovalDecision::Accept,
            feedback: None,
        },
    );

    let completions = outputs
        .iter()
        .filter(|output| matches!(output, ProviderOutput::TurnCompleted { .. }))
        .count();
    assert_eq!(completions, 1, "{outputs:#?}");
    let last = outputs.iter().rev().find_map(|output| match output {
        ProviderOutput::AssistantMessage { text, .. } => Some(text.clone()),
        _ => None,
    });
    assert!(
        last.is_some_and(|text| text.contains("ALPHA") && text.contains("BETA")),
        "{outputs:#?}"
    );
    let runs: Vec<_> = outputs
        .iter()
        .filter_map(|output| match output {
            ProviderOutput::RunEnded { .. } => Some("run ended"),
            ProviderOutput::BackgroundTaskEnded { .. } => Some("task ended"),
            ProviderOutput::AssistantMessage { .. } => Some("message"),
            _ => None,
        })
        .collect();
    let ended = runs.iter().filter(|kind| **kind == "task ended").count();
    assert_eq!(ended, 2, "{runs:?}");
    for (index, kind) in runs.iter().enumerate() {
        if *kind == "task ended" {
            assert_eq!(runs.get(index - 1), Some(&"run ended"), "{runs:?}");
        }
    }
    assert_eq!(runs.last(), Some(&"message"), "{runs:?}");
}

fn watched_request(cwd: &std::path::Path, prompt: &str, session: Option<String>) -> TurnRequest {
    TurnRequest {
        cwd: cwd.to_string_lossy().into_owned(),
        prompt: prompt.into(),
        attachments: Vec::new(),
        earlier_media: Vec::new(),
        resume_session_id: session,
        resume_session_at: None,
        model: None,
        effort: Some(hoshi_core::Effort::Low),
        fast_mode: false,
        ultracode: false,
        access_mode: AccessMode::FullAccess,
        plan_mode: false,
        mcp: None,
        instructions: None,
        subagent_effort: SubagentEffort::Thread,
        environment: Vec::new(),
    }
}

/// Starts a turn in `cwd`, hands each output to `act` as it comes, and collects everything
/// until the process is done: its turn completed without keeping it, or it ended.
fn run_watching(
    cwd: &std::path::Path,
    prompt: &str,
    mut act: impl FnMut(&ProviderOutput, &dyn hoshi_providers::RunningTurn),
) -> Vec<ProviderOutput> {
    let adapter = ClaudeCodeAdapter::new(None, Some("haiku".into()));
    let (sender, receiver) = mpsc::channel();
    let sink = Box::new(move |output| {
        let _ = sender.send(output);
    });
    let turn = adapter
        .start_turn(watched_request(cwd, prompt, None), sink)
        .unwrap();
    let mut outputs = Vec::new();
    for output in receiver.iter() {
        act(&output, turn.as_ref());
        let done = match output {
            ProviderOutput::TurnCompleted { .. } => !turn.lingers(),
            ProviderOutput::ProcessEnded => true,
            _ => false,
        };
        outputs.push(output);
        if done {
            break;
        }
    }
    outputs
}

fn last_roster_is_empty(outputs: &[ProviderOutput]) -> bool {
    outputs
        .iter()
        .rev()
        .find_map(|output| match output {
            ProviderOutput::BackgroundTasks { tasks, .. } => Some(tasks.is_empty()),
            _ => None,
        })
        .unwrap_or(false)
}

fn completions(outputs: &[ProviderOutput]) -> usize {
    outputs
        .iter()
        .filter(|output| matches!(output, ProviderOutput::TurnCompleted { .. }))
        .count()
}

/// Awaits the first background command as soon as it shows, as the agent would through
/// hoshi's tool.
fn await_first_command(output: &ProviderOutput, turn: &dyn hoshi_providers::RunningTurn) {
    if let ProviderOutput::BackgroundTasks { tasks, .. } = output
        && let Some(first) = tasks.first()
    {
        turn.await_background_tasks(std::slice::from_ref(&first.task_id), true);
    }
}

/// Writes `late.txt` unless it is stopped first.
const LATE_COMMAND: &str = "sleep 12 && echo late > late.txt";

fn start_late_command() -> String {
    format!(
        "Run the shell command `{LATE_COMMAND}` with the Bash tool using run_in_background: true. Then reply 'started' and end your turn."
    )
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn a_command_left_running_wakes_claude_in_a_turn_of_its_own_once_it_ends() {
    let cwd = scratch("background-command");
    let outputs = run_watching(
        &cwd,
        "Run the shell command `sleep 3 && echo done` with the Bash tool using run_in_background: true. Then reply 'started' and end your turn. When it finishes, tell me what it printed.",
        |_, _| {},
    );

    let first_end = outputs
        .iter()
        .position(|output| matches!(output, ProviderOutput::TurnCompleted { .. }))
        .unwrap_or_else(|| panic!("the turn never ended: {outputs:#?}"));
    let woke = outputs
        .iter()
        .position(|output| *output == ProviderOutput::Woke)
        .unwrap_or_else(|| panic!("Claude never woke: {outputs:#?}"));
    let task_ended = outputs
        .iter()
        .position(|output| {
            matches!(
                output,
                ProviderOutput::BackgroundTaskEnded { failed: false, .. }
            )
        })
        .unwrap_or_else(|| panic!("the command never ended: {outputs:#?}"));
    assert!(first_end < woke && woke < task_ended, "{outputs:#?}");
    assert!(last_roster_is_empty(&outputs), "{outputs:#?}");
    let answered = outputs[task_ended..].iter().any(|output| {
        matches!(output, ProviderOutput::AssistantMessage { text, .. } if text.contains("done"))
    });
    assert!(answered, "Claude never answered the command: {outputs:#?}");
    assert_eq!(completions(&outputs), 2, "{outputs:#?}");
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn an_awaited_background_command_holds_the_turn_until_claude_answers_it() {
    let cwd = scratch("awaited-background-command");
    let outputs = run_watching(
        &cwd,
        "Run the shell command `sleep 3 && echo done` with the Bash tool using run_in_background: true. Then reply 'started' and end your turn. When it finishes, tell me what it printed.",
        await_first_command,
    );

    let run_ended = outputs
        .iter()
        .position(|output| matches!(output, ProviderOutput::RunEnded { .. }))
        .unwrap_or_else(|| panic!("the turn ended with its first run: {outputs:#?}"));
    let task_ended = outputs
        .iter()
        .position(|output| matches!(output, ProviderOutput::BackgroundTaskEnded { .. }))
        .unwrap_or_else(|| panic!("the command never ended: {outputs:#?}"));
    assert!(run_ended < task_ended, "{outputs:#?}");
    assert!(!outputs.contains(&ProviderOutput::Woke), "{outputs:#?}");
    assert_eq!(completions(&outputs), 1, "{outputs:#?}");
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn a_stopped_command_left_running_ends_its_process() {
    let cwd = scratch("stop-background-command");
    let mut task: Option<String> = None;
    let mut stopped_at = None;
    let outputs = run_watching(&cwd, &start_late_command(), |output, turn| {
        if let ProviderOutput::BackgroundTasks { tasks, .. } = output
            && let Some(first) = tasks.first()
        {
            task = Some(first.task_id.clone());
        }
        if let (ProviderOutput::TurnCompleted { .. }, Some(task), None) =
            (output, &task, stopped_at)
        {
            turn.stop_background_task(task);
            stopped_at = Some(std::time::Instant::now());
        }
    });

    let since = stopped_at
        .unwrap_or_else(|| panic!("the turn kept nothing running: {outputs:#?}"))
        .elapsed();
    println!("ended {since:?} after the stop");
    assert!(since < std::time::Duration::from_secs(4), "{since:?}");
    assert!(last_roster_is_empty(&outputs), "{outputs:#?}");
    assert_eq!(outputs.last(), Some(&ProviderOutput::ProcessEnded));
    std::thread::sleep(std::time::Duration::from_secs(14));
    assert!(!cwd.join("late.txt").exists(), "the command kept running");
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn an_interrupt_ends_a_turn_that_waits_on_a_background_command() {
    let cwd = scratch("interrupt-background-command");
    let mut interrupted_at = None;
    let outputs = run_watching(&cwd, &start_late_command(), |output, turn| {
        await_first_command(output, turn);
        if matches!(output, ProviderOutput::RunEnded { .. }) && interrupted_at.is_none() {
            turn.interrupt();
            interrupted_at = Some(std::time::Instant::now());
        }
    });

    let since = interrupted_at
        .unwrap_or_else(|| panic!("the turn never waited on its command: {outputs:#?}"))
        .elapsed();
    println!("ended {since:?} after the interrupt");
    assert!(matches!(
        last_output(&outputs),
        Some(ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Interrupted
        })
    ));
    std::thread::sleep(std::time::Duration::from_secs(14));
    assert!(!cwd.join("late.txt").exists(), "the command kept running");
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn a_message_sent_while_a_background_command_runs_joins_the_turn() {
    let cwd = scratch("steer-background-command");
    let mut steered = false;
    let outputs = run_watching(
        &cwd,
        "Run the shell command `sleep 15 && echo done` with the Bash tool using run_in_background: true. Then reply 'started' and end your turn. When it finishes, tell me what it printed.",
        |output, turn| {
            await_first_command(output, turn);
            if matches!(output, ProviderOutput::RunEnded { .. }) && !steered {
                assert!(turn.steer(
                    "0f8d6a3c-2b1e-4c5d-9e7f-1a2b3c4d5e6f",
                    "While it runs, just say HOLA.",
                    &[],
                    true
                ));
                steered = true;
            }
        },
    );

    let hola = outputs.iter().position(|output| {
        matches!(output, ProviderOutput::AssistantMessage { text, .. } if text.contains("HOLA"))
    });
    let task_ended = outputs
        .iter()
        .position(|output| matches!(output, ProviderOutput::BackgroundTaskEnded { .. }));
    let hola = hola.unwrap_or_else(|| panic!("never said HOLA: {outputs:#?}"));
    let task_ended = task_ended.unwrap_or_else(|| panic!("the command never ended: {outputs:#?}"));
    assert!(
        hola < task_ended,
        "HOLA waited for the command: {outputs:#?}"
    );
    assert_eq!(completions(&outputs), 1, "{outputs:#?}");
    assert!(matches!(
        last_output(&outputs),
        Some(ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed
        })
    ));
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn the_next_message_runs_in_the_process_a_command_kept() {
    let cwd = scratch("continue-background-command");
    let mut session = None;
    let mut continued = false;
    let outputs = run_watching(
        &cwd,
        "Run the shell command `sleep 15 && echo done` with the Bash tool using run_in_background: true. Then reply 'started' and end your turn. When it finishes, tell me what it printed.",
        |output, turn| {
            if let ProviderOutput::SessionStarted { session_id } = output {
                session = Some(session_id.clone());
            }
            if matches!(output, ProviderOutput::TurnCompleted { .. }) && !continued {
                let request =
                    watched_request(&cwd, "While it runs, just say HOLA.", session.clone());
                assert!(turn.continue_with(&request));
                continued = true;
            }
        },
    );

    let hola = outputs.iter().position(|output| {
        matches!(output, ProviderOutput::AssistantMessage { text, .. } if text.contains("HOLA"))
    });
    let hola = hola.unwrap_or_else(|| panic!("never said HOLA: {outputs:#?}"));
    let task_ended = outputs
        .iter()
        .position(|output| matches!(output, ProviderOutput::BackgroundTaskEnded { .. }))
        .unwrap_or_else(|| panic!("the command never ended: {outputs:#?}"));
    assert!(hola < task_ended, "{outputs:#?}");
    assert!(completions(&outputs) >= 3, "{outputs:#?}");
}

/// As `run_watching`, but fails once the process is not done after `limit`, as a turn that
/// waits forever would.
fn run_watching_within(
    cwd: &std::path::Path,
    prompt: &str,
    limit: std::time::Duration,
    mut act: impl FnMut(&ProviderOutput, &dyn hoshi_providers::RunningTurn),
) -> Vec<ProviderOutput> {
    let adapter = ClaudeCodeAdapter::new(None, Some("haiku".into()));
    let (sender, receiver) = mpsc::channel();
    let sink = Box::new(move |output| {
        let _ = sender.send(output);
    });
    let turn = adapter
        .start_turn(watched_request(cwd, prompt, None), sink)
        .unwrap();
    let deadline = std::time::Instant::now() + limit;
    let mut outputs = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let Ok(output) = receiver.recv_timeout(left) else {
            turn.interrupt();
            panic!("the turn never ended: {outputs:#?}");
        };
        act(&output, turn.as_ref());
        let done = match output {
            ProviderOutput::TurnCompleted { .. } => !turn.lingers(),
            ProviderOutput::ProcessEnded => true,
            _ => false,
        };
        outputs.push(output);
        if done {
            return outputs;
        }
    }
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn a_background_command_of_a_background_subagent_never_leaves_the_turn_waiting() {
    let cwd = scratch("subagent-background-command");
    let mut asked = false;
    let outputs = run_watching_within(
        &cwd,
        "Delegate a task that waits 20 seconds in the background: launch ONE general-purpose subagent with the Agent tool using run_in_background: true. Tell the subagent to run the shell command `sleep 20` with the Bash tool using run_in_background: true, then wait for it to finish and reply 'waited'. After launching, reply 'launched' and end your turn. When the subagent finishes, tell me in one short sentence.",
        std::time::Duration::from_secs(150),
        |output, turn| {
            if matches!(output, ProviderOutput::RunEnded { .. }) && !asked {
                asked = turn.steer(
                    "2d7c4b1a-9e3f-4a6b-8c5d-0f1e2a3b4c5d",
                    "how are you?",
                    &[],
                    true,
                );
            }
        },
    );

    let sleep = outputs.iter().find_map(|output| match output {
        ProviderOutput::BackgroundTasks { tasks, .. } => {
            tasks.first().map(|task| task.task_id.clone())
        }
        _ => None,
    });
    let sleep =
        sleep.unwrap_or_else(|| panic!("the subagent ran no background command: {outputs:#?}"));
    let told_sleep = outputs.iter().any(|output| {
        matches!(output, ProviderOutput::BackgroundTaskEnded { task_id, .. } if *task_id == sleep)
    });
    assert!(
        !told_sleep,
        "the subagent's command woke the agent: {outputs:#?}"
    );
    assert!(last_roster_is_empty(&outputs), "{outputs:#?}");
    assert!(matches!(
        last_output(&outputs),
        Some(ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed
        })
    ));
}

#[test]
#[ignore = "spends tokens on a real Claude Code turn"]
fn a_background_command_that_ends_while_its_run_goes_on_is_answered_by_that_run() {
    let cwd = scratch("background-command-mid-run");
    let outputs = run_watching_within(
        &cwd,
        "First run the shell command `sleep 3 && echo early` with the Bash tool using run_in_background: true. Right after, run `sleep 8 && echo late` with the Bash tool in the FOREGROUND (no run_in_background). Then reply with one short sentence about both, and end your turn.",
        std::time::Duration::from_secs(90),
        |_, _| {},
    );

    assert!(
        outputs
            .iter()
            .any(|output| matches!(output, ProviderOutput::BackgroundTaskEnded { .. })),
        "{outputs:#?}"
    );
    assert_eq!(completions(&outputs), 1, "{outputs:#?}");
    assert!(matches!(
        last_output(&outputs),
        Some(ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed
        })
    ));
}

/// The last thing the turn reported, before its process ended.
fn last_output(outputs: &[ProviderOutput]) -> Option<&ProviderOutput> {
    outputs
        .iter()
        .rev()
        .find(|output| !matches!(output, ProviderOutput::ProcessEnded))
}
