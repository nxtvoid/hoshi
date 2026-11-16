//! Runs the real Codex CLI, signed in: `cargo test -p hoshi-providers --test codex_live --
//! --ignored --nocapture --test-threads=1`. `CODEX_BINARY` names the CLI, else `codex` on the
//! PATH. Spends requests.

use hoshi_core::{
    AccessMode, ApprovalDecision, Effort, ItemContent, ProviderAccountStatus, ProviderOutput,
    RequestResponse, SubagentEffort, TurnOutcome,
};
use hoshi_providers::{CodexAdapter, McpServer, ProviderAdapter, TitleRequest, TurnRequest};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

/// A quick model, so the turns stay cheap.
const MODEL: &str = "gpt-6-luna";

fn adapter() -> CodexAdapter {
    CodexAdapter::new(std::env::var("CODEX_BINARY").ok(), None)
}

/// A word no earlier run used, so Codex's memories of other threads can't answer for it.
fn secret() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    format!("kumquat{}", nanos % 100_000)
}

fn workspace(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("hoshi-codex-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.to_string_lossy().into_owned()
}

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
        model: Some(MODEL.into()),
        effort: Some(Effort::Low),
        fast_mode: false,
        ultracode: false,
        access_mode: AccessMode::FullAccess,
        plan_mode: false,
        mcp: None,
        instructions: Some("End every answer with the word over.".into()),
        subagent_effort: SubagentEffort::default(),
        environment: Vec::new(),
    }
}

/// Everything the turn reported, up to its end.
fn run(adapter: &CodexAdapter, request: TurnRequest) -> Vec<ProviderOutput> {
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
        .to_lowercase()
}

#[test]
#[ignore = "runs the real Codex CLI"]
fn a_turn_reads_a_file_and_the_next_one_resumes_its_thread() {
    let cwd = workspace("live");
    let word = secret();
    std::fs::write(
        std::path::Path::new(&cwd).join("notes.txt"),
        format!("the secret word is {word}\n"),
    )
    .unwrap();
    let adapter = adapter();

    let first = run(
        &adapter,
        request(
            &cwd,
            "Use a shell command to read notes.txt in the current directory, then tell me the secret word, one word only.",
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
    assert!(
        first
            .iter()
            .any(|output| matches!(output, ProviderOutput::ContextUsage(_)))
    );
    assert!(answer(&first).contains(&word));
    assert!(answer(&first).contains("over"));
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
    assert!(answer(&second).contains(&word));
}

#[test]
#[ignore = "runs the real Codex CLI"]
fn a_command_waits_for_approval_and_runs_once_accepted() {
    let cwd = workspace("ask");
    let word = secret();
    std::fs::write(
        std::path::Path::new(&cwd).join("notes.txt"),
        format!("the secret word is {word}\n"),
    )
    .unwrap();
    let adapter = adapter();
    let mut ask = request(
        &cwd,
        "Use a shell command to print notes.txt in the current directory, then tell me the secret word, one word only.",
        None,
    );
    ask.access_mode = AccessMode::Ask;
    let (sender, receiver) = mpsc::channel();
    let turn = adapter
        .start_turn(ask, Box::new(move |output| sender.send(output).unwrap()))
        .unwrap();

    let mut answered = 0;
    let mut answer = String::new();
    let outcome = loop {
        match receiver.recv_timeout(Duration::from_secs(180)).unwrap() {
            ProviderOutput::AssistantMessage { text, .. } => answer.push_str(&text),
            ProviderOutput::RequestOpened {
                request_id,
                content,
            } => {
                println!("{content:#?}");
                assert!(matches!(content, ItemContent::Approval { .. }));
                answered += 1;
                turn.respond(
                    &request_id,
                    &RequestResponse::Decision {
                        decision: ApprovalDecision::Accept,
                        feedback: None,
                    },
                );
            }
            ProviderOutput::TurnCompleted { outcome } => break outcome,
            output => println!("{output:?}"),
        }
    };
    println!("{outcome:?}");
    assert!(answered > 0);
    assert_eq!(outcome, TurnOutcome::Completed);
    assert!(answer.to_lowercase().contains(&word));
}

#[test]
#[ignore = "runs the real Codex CLI"]
fn a_stopped_turn_ends_interrupted() {
    let cwd = workspace("stop");
    let adapter = adapter();
    let (sender, receiver) = mpsc::channel();
    let turn = adapter
        .start_turn(
            request(
                &cwd,
                "Run the shell command `ping -n 30 127.0.0.1`, then reply done.",
                None,
            ),
            Box::new(move |output| sender.send(output).unwrap()),
        )
        .unwrap();
    loop {
        if let ProviderOutput::ToolCallStarted { .. } =
            receiver.recv_timeout(Duration::from_secs(180)).unwrap()
        {
            break;
        }
    }
    turn.interrupt();
    let outcome = loop {
        if let ProviderOutput::TurnCompleted { outcome } =
            receiver.recv_timeout(Duration::from_secs(60)).unwrap()
        {
            break outcome;
        }
    };
    assert_eq!(outcome, TurnOutcome::Interrupted);
}

#[test]
#[ignore = "runs the real Codex CLI"]
fn codex_names_a_thread_and_lists_its_models() {
    let adapter = adapter();

    let models = adapter.models();
    let title = adapter.generate_title(&TitleRequest {
        context: "the login button does nothing on mobile safari".into(),
        previous_title: None,
        images: Vec::new(),
        model: None,
        instructions: String::new(),
    });

    println!("{title:?}\n{models:#?}");
    assert!(title.is_some_and(|title| !title.title.is_empty()));
    assert!(!models.models.is_empty());
    assert!(models.default_model.is_some());
    assert!(models.models.iter().all(|model| !model.efforts.is_empty()));
}

#[test]
#[ignore = "runs the real Codex CLI"]
fn codex_reports_the_plans_limits_and_the_login() {
    let adapter = adapter();

    let limits = adapter.usage_limits();
    let status = adapter.account_status();
    let installation = adapter.installation();

    println!("{limits:#?}\n{status:?}\n{installation:?}");
    assert_eq!(status, ProviderAccountStatus::Ready);
    let limits = limits.unwrap();
    assert!(!limits.windows.is_empty());
    assert!(limits.account.is_some());
    assert!(installation.is_some_and(|installation| installation.installed.is_some()));
}

#[test]
#[ignore = "runs the real Codex CLI"]
fn a_plan_mode_turn_proposes_its_plan() {
    let cwd = workspace("plan");
    let adapter = adapter();
    let mut plan = request(
        &cwd,
        "Create a two-step plan to add a README to this folder. Do not ask questions. Present the final plan in a proposed plan block.",
        None,
    );
    plan.plan_mode = true;
    let (sender, receiver) = mpsc::channel();
    let turn = adapter
        .start_turn(plan, Box::new(move |output| sender.send(output).unwrap()))
        .unwrap();

    let mut proposed = None;
    let outcome = loop {
        match receiver.recv_timeout(Duration::from_secs(180)).unwrap() {
            ProviderOutput::RequestOpened {
                request_id,
                content: ItemContent::ProposedPlan { plan, .. },
            } => {
                println!("{plan}");
                proposed = Some(plan);
                turn.respond(
                    &request_id,
                    &RequestResponse::Decision {
                        decision: ApprovalDecision::Decline,
                        feedback: None,
                    },
                );
            }
            ProviderOutput::TurnCompleted { outcome } => break outcome,
            _ => {}
        }
    };
    assert!(proposed.is_some_and(|plan| plan.to_lowercase().contains("readme")));
    assert_eq!(outcome, TurnOutcome::Completed);
    assert!(!std::path::Path::new(&cwd).join("README.md").exists());
}

#[test]
#[ignore = "runs the real Codex CLI"]
fn a_running_turn_takes_another_message() {
    let cwd = workspace("steer");
    let adapter = adapter();
    let (sender, receiver) = mpsc::channel();
    let turn = adapter
        .start_turn(
            request(
                &cwd,
                "Run the shell command `ping -n 6 127.0.0.1`, then reply done.",
                None,
            ),
            Box::new(move |output| sender.send(output).unwrap()),
        )
        .unwrap();
    let mut steered = false;
    let mut answer = String::new();
    let outcome = loop {
        match receiver.recv_timeout(Duration::from_secs(180)).unwrap() {
            ProviderOutput::ToolCallStarted { .. } if !steered => {
                steered = turn.steer("m2", "Also end your reply with the word papaya.", &[], true);
            }
            ProviderOutput::AssistantMessage { text, .. } => answer.push_str(&text),
            ProviderOutput::TurnCompleted { outcome } => break outcome,
            _ => {}
        }
    };
    println!("{answer}");
    assert!(steered);
    assert_eq!(outcome, TurnOutcome::Completed);
    assert!(answer.to_lowercase().contains("papaya"));
}

#[test]
#[ignore = "runs the real Codex CLI"]
fn a_rewind_resumes_the_thread_without_what_came_after() {
    let cwd = workspace("rewind");
    let adapter = adapter();
    let cursor = |outputs: &[ProviderOutput]| {
        outputs.iter().find_map(|output| match output {
            ProviderOutput::ConversationCursor { cursor } => Some(cursor.clone()),
            _ => None,
        })
    };
    let session = |outputs: &[ProviderOutput]| {
        outputs.iter().find_map(|output| match output {
            ProviderOutput::SessionStarted { session_id } => Some(session_id.clone()),
            _ => None,
        })
    };

    let first = run(
        &adapter,
        request(&cwd, "Remember the word lemon. Reply ok.", None),
    );
    let second = run(
        &adapter,
        request(
            &cwd,
            "Also remember the word cherry. Reply ok.",
            session(&first),
        ),
    );
    let mut rewound = request(
        &cwd,
        "Which words did I ask you to remember? Answer with the words only, no tools.",
        session(&second),
    );
    rewound.resume_session_at = cursor(&first);
    let third = run(&adapter, rewound);

    println!("{}", answer(&third));
    assert!(cursor(&first).is_some());
    assert_ne!(session(&third), session(&first));
    assert!(answer(&third).contains("lemon"));
    assert!(!answer(&third).contains("cherry"));
}

#[test]
#[ignore = "runs the real Codex CLI"]
fn compact_compacts_the_thread() {
    let cwd = workspace("compact");
    let adapter = adapter();
    let first = run(&adapter, request(&cwd, "Reply ok.", None));
    let session = first.iter().find_map(|output| match output {
        ProviderOutput::SessionStarted { session_id } => Some(session_id.clone()),
        _ => None,
    });

    let compacted = run(&adapter, request(&cwd, "/compact", session));

    println!("{compacted:#?}");
    assert!(compacted.contains(&ProviderOutput::CompactionStarted));
    assert!(
        compacted
            .iter()
            .any(|output| matches!(output, ProviderOutput::CompactionFinished { .. }))
    );
    assert_eq!(
        compacted.last(),
        Some(&ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed
        })
    );
}

#[test]
#[ignore = "runs the real Codex CLI"]
fn a_subagent_runs_under_the_call_that_started_it() {
    let cwd = workspace("subagent");
    let adapter = adapter();

    let outputs = run(
        &adapter,
        request(
            &cwd,
            "Spawn one subagent whose only task is to reply with exactly: Hello. Wait for it to finish, then reply with exactly what it said.",
            None,
        ),
    );

    println!("{outputs:#?}");
    let call = outputs
        .iter()
        .find_map(|output| match output {
            ProviderOutput::ToolCallStarted { item_id, name, .. } if name == "Task" => {
                Some(item_id.clone())
            }
            _ => None,
        })
        .expect("a Task call");
    assert!(outputs.iter().any(|output| matches!(
        output,
        ProviderOutput::Subagent { tool_use_id, output }
            if *tool_use_id == call && matches!(**output, ProviderOutput::AssistantMessage { .. })
    )));
    assert!(outputs.iter().any(|output| matches!(
        output,
        ProviderOutput::ToolCallCompleted { item_id, output, .. }
            if *item_id == call && output.contains("Hello")
    )));
}

/// The reply to one request of MCP's streamable HTTP, for a server with one tool,
/// `echo_text`; `None` for what isn't a JSON-RPC post.
fn mcp_reply(post: bool, message: &Value) -> Option<String> {
    if !post {
        return None;
    }
    let result = match message["method"].as_str() {
        _ if message.get("id").is_none() => return Some(String::new()),
        Some("initialize") => json!({
            "protocolVersion": message["params"]["protocolVersion"],
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "hoshi", "version": "1" },
        }),
        Some("tools/list") => json!({ "tools": [{
            "name": "echo_text",
            "description": "Echoes the text back, uppercased.",
            "inputSchema": {
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"],
            },
        }] }),
        Some("tools/call") => {
            let text = message["params"]["arguments"]["text"]
                .as_str()
                .unwrap_or_default();
            json!({ "content": [{ "type": "text", "text": text.to_uppercase() }] })
        }
        _ => json!({}),
    };
    Some(json!({ "jsonrpc": "2.0", "id": message["id"], "result": result }).to_string())
}

/// Serves [`mcp_reply`] over HTTP; the receiver gets the `Authorization` of each tool call.
fn mcp_server() -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let (sender, calls) = mpsc::channel();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let sender = sender.clone();
            std::thread::spawn(move || {
                let mut writer = stream.try_clone().unwrap();
                let mut reader = BufReader::new(stream);
                loop {
                    let mut request_line = String::new();
                    if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
                        return;
                    }
                    let (mut length, mut authorization) = (0, String::new());
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap_or(0) == 0 {
                            return;
                        }
                        let Some((name, value)) = header.trim_end().split_once(':') else {
                            break;
                        };
                        match name.to_ascii_lowercase().as_str() {
                            "content-length" => length = value.trim().parse().unwrap_or(0),
                            "authorization" => value.trim().clone_into(&mut authorization),
                            _ => {}
                        }
                    }
                    let mut body = vec![0; length];
                    if reader.read_exact(&mut body).is_err() {
                        return;
                    }
                    let message: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    if message["method"] == "tools/call" {
                        let _ = sender.send(authorization);
                    }
                    let response = match mcp_reply(request_line.starts_with("POST"), &message) {
                        None => "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n"
                            .to_owned(),
                        Some(body) if body.is_empty() => {
                            "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n".to_owned()
                        }
                        Some(body) => format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                            body.len()
                        ),
                    };
                    if writer.write_all(response.as_bytes()).is_err() {
                        return;
                    }
                }
            });
        }
    });
    (url, calls)
}

#[test]
#[ignore = "runs the real Codex CLI"]
fn hoshis_mcp_tools_run_without_asking() {
    let cwd = workspace("mcp");
    let (url, calls) = mcp_server();
    let adapter = adapter();
    let mut call = request(
        &cwd,
        "Call the echo_text tool of the hoshi MCP server with the text kiwi, then reply with its result only.",
        None,
    );
    call.access_mode = AccessMode::Ask;
    call.mcp = Some(McpServer {
        url,
        authorization: "Bearer thread-1".into(),
    });

    let outputs = run(&adapter, call);

    println!("{outputs:#?}");
    assert!(outputs.iter().any(|output| matches!(
        output,
        ProviderOutput::ToolCallStarted { name, .. } if name == "mcp__hoshi__echo_text"
    )));
    assert!(
        !outputs
            .iter()
            .any(|output| matches!(output, ProviderOutput::RequestOpened { .. }))
    );
    assert_eq!(calls.try_recv().as_deref(), Ok("Bearer thread-1"));
    assert!(answer(&outputs).contains("kiwi"));
}
