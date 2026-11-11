//! A thread's real Claude Code agent leaves a server running in the background, started
//! through a shell script as `npm run dev` starts one. Stopping that command from the thread
//! ends the server too, and the agent stays at rest: it is not told the command failed.
//! Costs a few cents, so it only runs on request:
//! `cargo test -p hoshi-engine --test servers_live -- --ignored --nocapture`

use hoshi_core::{AccessMode, Command, Event, ItemContent, ThreadWorkspace, TurnOutcome};
use hoshi_engine::{Engine, Notification, Notify};
use hoshi_providers::ClaudeCodeAdapter;
use hoshi_store::Store;
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

fn listening(port: u16) -> bool {
    TcpStream::connect_timeout(
        &(Ipv4Addr::LOCALHOST, port).into(),
        Duration::from_millis(300),
    )
    .is_ok()
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn stopping_a_background_command_ends_its_server_and_leaves_the_agent_at_rest() {
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let root = std::env::temp_dir().join(format!("hoshi-servers-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("server.js"),
        format!(
            "require('http').createServer((q, s) => s.end('ok')).listen({port}, () => console.log('ready on http://localhost:{port}'))\n"
        ),
    )
    .unwrap();
    let script = project.join("dev.sh");
    std::fs::write(&script, "#!/bin/sh\nnode server.js\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let (sender, events) = mpsc::channel();
    let running = Arc::new(Mutex::new(Vec::new()));
    let roster = running.clone();
    let notify: Notify = Arc::new(move |notification| match notification {
        Notification::Event(envelope) => {
            let _ = sender.send(envelope.event);
        }
        Notification::BackgroundTasks(background) => {
            *roster.lock().unwrap() = background.tasks;
        }
        _ => {}
    });
    let engine =
        Engine::from_store(Store::open_in_memory().unwrap(), &root.join("data"), notify).unwrap();
    let model = std::env::var("HOSHI_LIVE_MODEL").unwrap_or_else(|_| "sonnet".into());
    engine.set_adapter(Some(Arc::new(ClaudeCodeAdapter::new(None, Some(model)))));
    engine
        .dispatch(Command::ProjectCreate {
            project_id: "p".into(),
            title: "Project".into(),
            workspace_root: project.to_string_lossy().into_owned(),
        })
        .unwrap();
    engine
        .dispatch(Command::ThreadCreate {
            thread_id: "t".into(),
            project_id: Some("p".into()),
            title: "Dev server".into(),
            workspace: ThreadWorkspace::Local,
            model: None,
            effort: Some(hoshi_core::Effort::Low),
            fast_mode: false,
            ultracode: false,
            access_mode: AccessMode::FullAccess,
            plan_mode: false,
        })
        .unwrap();
    engine
        .dispatch(Command::MessageSend {
            thread_id: "t".into(),
            message_id: "33333333-3333-4333-8333-333333333333".into(),
            text: "levanta el server de dev en segundo plano con la herramienta Bash, con este comando tal cual, y avisame cuando responda:\n\n./dev.sh".into(),
            attachments: Vec::new(),
        })
        .unwrap();

    loop {
        match events.recv_timeout(Duration::from_secs(180)) {
            Ok(Event::TurnCompleted { outcome, .. }) => {
                assert_eq!(outcome, TurnOutcome::Completed);
                break;
            }
            Ok(Event::ItemAdded { item }) => {
                if let ItemContent::ToolCall { name, input, .. } = item.content {
                    eprintln!("{name} {input}");
                }
            }
            Ok(_) => {}
            Err(_) => panic!("timed out"),
        }
    }
    assert!(listening(port), "the agent left no server on {port}");
    let tasks = running.lock().unwrap().clone();
    let [task] = tasks.as_slice() else {
        panic!("not one background command: {tasks:?}");
    };

    engine
        .dispatch(Command::BackgroundTaskStop {
            thread_id: "t".into(),
            task_id: task.task_id.clone(),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(12);
    while listening(port) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
    }
    let left_running = listening(port);

    let mut after = Vec::new();
    let quiet_until = Instant::now() + Duration::from_secs(25);
    while let Some(left) = quiet_until.checked_duration_since(Instant::now()) {
        match events.recv_timeout(left) {
            Ok(Event::TurnStarted { .. }) => after.push("the agent started a turn".to_owned()),
            Ok(Event::ItemAdded { item }) => match item.content {
                ItemContent::BackgroundTaskEnded { failed, .. } => {
                    after.push(format!("a background task ended, failed: {failed}"))
                }
                ItemContent::ToolCall { name, input, .. } => after.push(format!("{name} {input}")),
                ItemContent::AssistantMessage { text } => after.push(text),
                _ => {}
            },
            Ok(_) => {}
            Err(_) => break,
        }
    }
    eprintln!("--- after the stop\n{}", after.join("\n"));
    // Whatever the stop left behind is no longer anyone's; end it by its port.
    #[cfg(windows)]
    if listening(port) {
        let end = format!(
            "Get-NetTCPConnection -LocalPort {port} | ForEach-Object {{ Stop-Process -Id $_.OwningProcess -Force }}"
        );
        let _ = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", &end])
            .output();
    }

    assert!(!left_running, "the server still listens after the stop");
    assert!(after.is_empty(), "the agent went on after the stop");
}
