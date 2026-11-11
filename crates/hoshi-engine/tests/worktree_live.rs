//! Asked to move to a worktree that already exists, or only to use it, a thread's real
//! Claude Code agent takes its session there, so the thread follows. Costs a few cents, so
//! it only runs on request:
//! `cargo test -p hoshi-engine --test worktree_live -- --ignored`

use hoshi_core::{AccessMode, Command, Event, ItemContent, ThreadWorkspace, TurnOutcome};
use hoshi_engine::{Engine, Notification, Notify};
use hoshi_providers::ClaudeCodeAdapter;
use hoshi_store::Store;
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

const BRANCH: &str = "OE-6682-error-exporting";

fn git(dir: &Path, arguments: &[&str]) {
    let status = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=hoshi",
            "-c",
            "user.email=hoshi@example.com",
        ])
        .args(arguments)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&status.stderr)
    );
}

/// Sends `prompt` to a thread on the project root; `{branch}` in it names a branch that
/// has a worktree next to the project.
fn thread_follows(name: &str, prompt: &str) {
    let root = std::env::temp_dir().join(format!("hoshi-worktree-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("shop");
    let worktree = root.join("shop-OE-6682");
    std::fs::create_dir_all(&project).unwrap();
    git(&project, &["init", "-b", "main"]);
    std::fs::write(project.join("README.md"), "shop\n").unwrap();
    git(&project, &["add", "."]);
    git(&project, &["commit", "-m", "start"]);
    git(
        &project,
        &["worktree", "add", "-b", BRANCH, &worktree.to_string_lossy()],
    );
    std::fs::write(worktree.join("export.txt"), "orders\n").unwrap();
    git(&worktree, &["add", "."]);
    git(&worktree, &["commit", "-m", "export orders"]);

    let (sender, events) = mpsc::channel();
    let notify: Notify = Arc::new(move |notification| {
        if let Notification::Event(envelope) = notification {
            let _ = sender.send(envelope.event);
        }
    });
    let engine =
        Engine::from_store(Store::open_in_memory().unwrap(), &root.join("data"), notify).unwrap();
    let model = std::env::var("HOSHI_LIVE_MODEL").unwrap_or_else(|_| "sonnet".into());
    engine.set_adapter(Some(Arc::new(ClaudeCodeAdapter::new(None, Some(model)))));
    engine
        .dispatch(Command::ProjectCreate {
            project_id: "p".into(),
            title: "Shop".into(),
            workspace_root: project.to_string_lossy().into_owned(),
        })
        .unwrap();
    engine
        .dispatch(Command::ThreadCreate {
            thread_id: "t".into(),
            project_id: Some("p".into()),
            title: "Export".into(),
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
            message_id: "22222222-2222-4222-8222-222222222222".into(),
            text: prompt.replace("{branch}", BRANCH),
            attachments: Vec::new(),
        })
        .unwrap();

    loop {
        match events.recv_timeout(Duration::from_secs(180)) {
            Ok(Event::TurnCompleted { outcome, .. }) => {
                assert_eq!(outcome, TurnOutcome::Completed);
                break;
            }
            Ok(_) => {}
            Err(_) => panic!("timed out"),
        }
    }

    let thread = || {
        engine
            .snapshot()
            .threads
            .into_iter()
            .find(|thread| thread.id == "t")
            .unwrap()
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    while thread().worktree_path.is_none() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let thread = thread();
    let did: Vec<String> = engine
        .transcript("t", None)
        .unwrap()
        .items
        .into_iter()
        .filter_map(|item| match item.content {
            ItemContent::ToolCall { name, input, .. } => Some(format!("> {name} {input}")),
            ItemContent::AssistantMessage { text } => Some(text),
            _ => None,
        })
        .collect();
    let did = did.join("\n");
    eprintln!("--- {name}\n{did}");
    assert!(
        thread
            .worktree_path
            .as_deref()
            .is_some_and(|path| path.ends_with("shop-OE-6682")),
        "{name}: {:?}",
        thread.worktree_path
    );
    assert_eq!(thread.branch.as_deref(), Some(BRANCH));
    assert!(thread.worktree_adopted);
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn an_agent_asked_to_move_to_a_worktree_takes_the_thread_there() {
    thread_follows(
        "move",
        "hay un worktree de esta rama \"{branch}\" buscalo y muevete a el pls y dime cual es su ultimo commit",
    );
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn an_agent_asked_to_use_a_worktree_takes_the_thread_there() {
    thread_follows(
        "use",
        "existe algun worktree para la rama \"{branch}\"?, usalo",
    );
}

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn an_agent_asked_to_go_on_in_a_branch_with_a_worktree_takes_the_thread_there() {
    thread_follows(
        "go-on",
        "sigamos con la rama \"{branch}\", creo que ya tiene un worktree. agrega una linea \"invoices\" a export.txt",
    );
}
