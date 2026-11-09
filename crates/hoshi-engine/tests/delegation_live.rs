//! A thread's real Claude Code agent delegates a task to a child thread through hoshi's MCP
//! server, and gets its answer back. Costs a few cents, so it only runs on request:
//! `cargo test -p hoshi-engine --test delegation_live -- --ignored`

use hoshi_core::{AccessMode, Command, Event, ItemContent, ThreadWorkspace, TurnOutcome};
use hoshi_engine::{Engine, Notification, Notify};
use hoshi_providers::ClaudeCodeAdapter;
use hoshi_store::Store;
use std::sync::{Arc, mpsc};
use std::time::Duration;

#[test]
#[ignore = "spends tokens on real Claude Code turns"]
fn an_agent_delegates_a_task_and_gets_its_answer_back() {
    let root = std::env::temp_dir().join(format!("hoshi-delegation-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("project")).unwrap();
    let (sender, events) = mpsc::channel();
    let notify: Notify = Arc::new(move |notification| {
        if let Notification::Event(envelope) = notification {
            let _ = sender.send(envelope.event);
        }
    });
    let engine =
        Engine::from_store(Store::open_in_memory().unwrap(), &root.join("data"), notify).unwrap();
    engine.set_adapter(Some(Arc::new(ClaudeCodeAdapter::new(
        None,
        Some("haiku".into()),
    ))));
    engine
        .dispatch(Command::ProjectCreate {
            project_id: "p".into(),
            title: "Project".into(),
            workspace_root: root.join("project").to_string_lossy().into_owned(),
        })
        .unwrap();
    engine
        .dispatch(Command::ThreadCreate {
            thread_id: "t".into(),
            project_id: Some("p".into()),
            title: "Parent".into(),
            workspace: ThreadWorkspace::Local,
            model: None,
            effort: Some(hoshi_core::Effort::Low),
            fast_mode: false,
            ultracode: false,
            access_mode: AccessMode::AcceptEdits,
            plan_mode: false,
        })
        .unwrap();
    engine
        .dispatch(Command::MessageSend {
            thread_id: "t".into(),
            message_id: "11111111-1111-4111-8111-111111111111".into(),
            text: "Use the hoshi delegate_task tool to delegate this task: \"Reply with exactly the word PINEAPPLE.\" Then end your turn. When the task's answer arrives, tell me the word it answered.".into(),
            attachments: Vec::new(),
        })
        .unwrap();

    let mut ended = Vec::new();
    while ended
        .iter()
        .filter(|thread_id: &&String| *thread_id == "t")
        .count()
        < 2
    {
        match events.recv_timeout(Duration::from_secs(180)) {
            Ok(Event::TurnCompleted {
                thread_id, outcome, ..
            }) => {
                assert_eq!(outcome, TurnOutcome::Completed, "{thread_id}");
                ended.push(thread_id);
            }
            Ok(_) => {}
            Err(_) => panic!("timed out; turns ended so far: {ended:?}"),
        }
    }

    let snapshot = engine.snapshot();
    let child = snapshot
        .threads
        .iter()
        .find(|thread| thread.parent_thread_id.as_deref() == Some("t"))
        .expect("a child thread");
    assert!(ended.contains(&child.id), "{ended:?}");
    let items = engine.transcript("t", None).unwrap().items;
    let delivered = items.iter().any(|item| {
        matches!(&item.content, ItemContent::UserMessage { task_result: Some(from), text, .. }
            if *from == child.id && text.contains("PINEAPPLE"))
    });
    assert!(delivered, "{items:#?}");
    let answered = items.iter().rev().find_map(|item| match &item.content {
        ItemContent::AssistantMessage { text } => Some(text.clone()),
        _ => None,
    });
    assert!(
        answered.is_some_and(|text| text.contains("PINEAPPLE")),
        "{items:#?}"
    );
}
