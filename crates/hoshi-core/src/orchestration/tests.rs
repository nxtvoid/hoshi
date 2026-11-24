use super::*;
use crate::Question;

const NOW: &str = "2026-11-07T12:00:00.000Z";

fn run(model: &mut ReadModel, command: Command) -> Result<Vec<Event>, CommandError> {
    let events = decide(model, command, NOW)?;
    for event in &events {
        model.apply(event);
    }
    Ok(events)
}

fn record(model: &mut ReadModel, turn_id: &str, output: ProviderOutput) -> Vec<Event> {
    let events = record_output(model, "t", turn_id, output, NOW);
    for event in &events {
        model.apply(event);
    }
    events
}

fn create_project(id: &str, root: &str) -> Command {
    Command::ProjectCreate {
        project_id: id.into(),
        title: id.into(),
        workspace_root: root.into(),
    }
}

fn create_thread(id: &str, project_id: &str) -> Command {
    create_thread_in(id, Some(project_id))
}

fn create_thread_in(id: &str, project_id: Option<&str>) -> Command {
    Command::ThreadCreate {
        thread_id: id.into(),
        project_id: project_id.map(str::to_owned),
        title: String::new(),
        workspace: ThreadWorkspace::Local,
        provider: crate::default_provider(),
        model: None,
        effort: None,
        fast_mode: false,
        ultracode: false,
        access_mode: AccessMode::AcceptEdits,
        plan_mode: false,
    }
}

fn send(message_id: &str, text: &str) -> Command {
    Command::MessageSend {
        thread_id: "t".into(),
        message_id: message_id.into(),
        text: text.into(),
        attachments: Vec::new(),
    }
}

fn compaction_status(events: &[Event]) -> Option<CompactionStatus> {
    events.iter().find_map(|event| match event {
        Event::ItemAdded {
            item:
                ThreadItem {
                    content: ItemContent::Compaction { status, .. },
                    ..
                },
        } => Some(*status),
        _ => None,
    })
}

#[test]
fn compact_shows_its_compaction_at_once_and_ends_it_with_the_turn() {
    let mut model = model_with_thread();
    let sent = run(&mut model, send("c1", "/compact")).unwrap();
    assert_eq!(compaction_status(&sent), Some(CompactionStatus::Running));

    let done = record(
        &mut model,
        "c1",
        ProviderOutput::CompactionFinished {
            before_tokens: 150_000,
            after_tokens: 12_000,
        },
    );
    assert_eq!(compaction_status(&done), Some(CompactionStatus::Done));
    let ended = record(
        &mut model,
        "c1",
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        },
    );
    assert_eq!(compaction_status(&ended), None);

    run(&mut model, send("c2", "/compact")).unwrap();
    let ended = record(
        &mut model,
        "c2",
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Interrupted,
        },
    );
    assert_eq!(compaction_status(&ended), Some(CompactionStatus::Failed));

    let sent = run(&mut model, send("m1", "carry on")).unwrap();
    assert_eq!(compaction_status(&sent), None);
    record(
        &mut model,
        "m1",
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        },
    );

    let queued = ThreadItem {
        id: "c3".into(),
        thread_id: "t".into(),
        created_at: NOW.into(),
        content: ItemContent::UserMessage {
            text: "/compact".into(),
            attachments: Vec::new(),
            steer: true,
            task_result: None,
            pull_request_update: None,
            resumed: None,
            rewind: None,
            dropped: false,
            model_switch: None,
        },
    };
    let started = start_queued_turn(&model, "t", "c3", Some(&queued), NOW);
    assert_eq!(compaction_status(&started), Some(CompactionStatus::Running));
}

#[test]
fn task_preview_keeps_one_short_line() {
    assert_eq!(
        task_preview(
            "  Found it.

- one
- two  "
        ),
        "Found it. - one - two"
    );
    let long = "word ".repeat(100);
    let preview = task_preview(&long);
    assert_eq!(preview.chars().count(), TASK_PREVIEW_CHARS);
    assert!(preview.ends_with("word…"));
}

#[test]
fn keeps_the_latest_message_of_delegated_tasks_only() {
    let mut model = model_with_thread();
    run(&mut model, create_thread("task", "p")).unwrap();
    model.apply(&Event::ThreadDelegated {
        thread_id: "task".into(),
        parent_thread_id: "t".into(),
        shares_worktree: false,
    });
    let say = |thread_id: &str, id: &str, text: &str| Event::ItemAdded {
        item: ThreadItem {
            id: id.into(),
            thread_id: thread_id.into(),
            created_at: NOW.into(),
            content: ItemContent::AssistantMessage { text: text.into() },
        },
    };

    model.apply(&say("t", "m1", "the parent talks"));
    model.apply(&say("task", "m2", "Looking"));
    model.apply(&say(
        "task",
        "m3",
        "Counted
42 files",
    ));

    assert_eq!(
        model.task_previews(),
        [TaskPreview {
            thread_id: "task".into(),
            text: "Counted 42 files".into(),
        }]
    );
    model.apply(&Event::ThreadDeleted {
        thread_id: "task".into(),
    });
    assert!(model.task_previews().is_empty());
}

fn model_with_thread() -> ReadModel {
    let mut model = ReadModel::default();
    run(&mut model, create_project("p", "/repo")).unwrap();
    run(&mut model, create_thread("t", "p")).unwrap();
    model
}

#[test]
fn rejects_a_second_project_on_the_same_folder() {
    let mut model = ReadModel::default();
    run(&mut model, create_project("a", "/repo/hoshi")).unwrap();

    let error = run(&mut model, create_project("b", "/repo/hoshi/")).unwrap_err();

    assert_eq!(
        error,
        CommandError::DuplicateWorkspaceRoot {
            project_id: "a".into()
        }
    );
}

#[test]
fn rejects_a_thread_for_a_missing_project() {
    let mut model = ReadModel::default();

    let error = run(&mut model, create_thread("t", "nope")).unwrap_err();

    assert_eq!(
        error,
        CommandError::ProjectNotFound {
            project_id: "nope".into()
        }
    );
}

#[test]
fn a_chat_belongs_to_no_project_and_works_only_locally() {
    let mut model = ReadModel::default();
    run(&mut model, create_project("p", "/repo")).unwrap();
    run(&mut model, create_thread_in("chat", None)).unwrap();

    assert_eq!(model.thread("chat").unwrap().project_id, None);
    assert_eq!(model.thread_cwd("chat"), None);

    let mut worktree = create_thread_in("c2", None);
    if let Command::ThreadCreate { workspace, .. } = &mut worktree {
        *workspace = ThreadWorkspace::Worktree {
            path: "/wt".into(),
            branch: "b".into(),
        };
    }
    assert_eq!(
        run(&mut model, worktree).unwrap_err(),
        CommandError::ChatNotLocal {
            thread_id: "c2".into()
        }
    );

    run(
        &mut model,
        Command::ProjectRemove {
            project_id: "p".into(),
        },
    )
    .unwrap();
    assert!(model.thread("chat").is_some());
}

#[test]
fn removing_a_project_removes_its_threads_and_frees_the_folder() {
    let mut model = ReadModel::default();
    run(&mut model, create_project("p", "/repo")).unwrap();
    run(&mut model, create_project("q", "/other")).unwrap();
    run(&mut model, create_thread("t1", "p")).unwrap();
    run(&mut model, create_thread("t2", "q")).unwrap();

    run(
        &mut model,
        Command::ProjectRemove {
            project_id: "p".into(),
        },
    )
    .unwrap();

    let thread_ids: Vec<_> = model.threads().map(|thread| thread.id.as_str()).collect();
    assert_eq!(thread_ids, ["t2"]);
    run(&mut model, create_project("p2", "/repo")).unwrap();
}

#[test]
fn first_message_titles_the_thread_and_starts_a_turn() {
    let mut model = model_with_thread();

    run(&mut model, send("m1", "  Fix the login bug\nwith details")).unwrap();

    let thread = model.thread("t").unwrap();
    assert_eq!(thread.title, "Fix the login bug");
    assert_eq!(thread.status, ThreadStatus::Working);
    assert_eq!(thread.active_turn_id.as_deref(), Some("m1"));
}

#[test]
fn the_first_title_skips_markdown_syntax_and_code() {
    let title = |text: &str| title_from(text, &[]);

    assert_eq!(
        title(
            "```tsx
const a = 1
```

## Fix the *login* `bug`"
        ),
        "Fix the login bug"
    );
    assert_eq!(title("> quoted ask"), "quoted ask");
    assert_eq!(
        title(
            "1. first step
2. second"
        ),
        "first step"
    );
    assert_eq!(
        title(
            "```tsx
```"
        ),
        ""
    );
}

#[test]
fn a_generated_title_replaces_only_the_title_it_started_from() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "fix the login bug")).unwrap();

    assert!(record_title(&model, "t", "something else", "Login Bug").is_empty());
    assert!(record_title(&model, "t", "fix the login bug", "  ").is_empty());
    let events = record_title(&model, "t", "fix the login bug", " Fix Login Redirect ");

    assert_eq!(
        events,
        [Event::ThreadTitleSet {
            thread_id: "t".into(),
            title: "Fix Login Redirect".into()
        }]
    );
}

#[test]
fn a_message_to_a_busy_thread_joins_its_turn() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "first")).unwrap();

    let events = run(&mut model, send("m2", "second")).unwrap();

    assert!(matches!(
        events.as_slice(),
        [Event::ItemAdded { item }] if item.id == "m2"
            && matches!(item.content, ItemContent::UserMessage { steer: true, .. })
    ));
    assert_eq!(
        model.thread("t").unwrap().active_turn_id.as_deref(),
        Some("m1")
    );
    assert!(start_queued_turn(&model, "t", "m2", None, NOW).is_empty());

    record(
        &mut model,
        "m1",
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        },
    );
    assert!(matches!(
        start_queued_turn(&model, "t", "m2", None, NOW).as_slice(),
        [Event::TurnStarted { turn_id, .. }] if turn_id == "m2"
    ));
}

#[test]
fn rejects_an_empty_message() {
    let mut model = model_with_thread();

    assert_eq!(
        run(&mut model, send("m1", "   ")).unwrap_err(),
        CommandError::EmptyMessage
    );
}

fn image(name: &str) -> MessageAttachment {
    MessageAttachment::Image {
        id: "i1".into(),
        name: name.into(),
        mime_type: "image/png".into(),
        size: 10,
    }
}

#[test]
fn an_image_alone_is_a_message_named_after_it() {
    let mut model = model_with_thread();
    let command = Command::MessageSend {
        thread_id: "t".into(),
        message_id: "m1".into(),
        text: " ".into(),
        attachments: vec![image("screenshot.png")],
    };

    let events = run(&mut model, command).unwrap();

    assert_eq!(model.thread("t").unwrap().title, "screenshot.png");
    assert!(events.iter().any(|event| matches!(
        event,
        Event::ItemAdded { item } if item.content == ItemContent::UserMessage {
            text: String::new(),
            attachments: vec![image("screenshot.png")],
            steer: false,
            task_result: None,
            pull_request_update: None,
            resumed: None,
            rewind: Some(Rewind::Start),
            dropped: false,
            model_switch: None,
        }
    )));
}

#[test]
fn a_file_alone_is_a_message_named_after_it() {
    let mut model = model_with_thread();
    let file = MessageAttachment::File {
        id: "f1".into(),
        name: "report.pdf".into(),
        mime_type: "application/pdf".into(),
        size: 10,
    };
    let command = Command::MessageSend {
        thread_id: "t".into(),
        message_id: "m1".into(),
        text: String::new(),
        attachments: vec![file],
    };

    run(&mut model, command).unwrap();

    assert_eq!(model.thread("t").unwrap().title, "report.pdf");
}

fn rewind_of(events: &[Event]) -> Option<Rewind> {
    events.iter().find_map(|event| match event {
        Event::ItemAdded { item } => match &item.content {
            ItemContent::UserMessage { rewind, .. } => Some(rewind.clone()),
            _ => None,
        },
        _ => None,
    })?
}

fn finish_turn(model: &mut ReadModel, turn_id: &str) {
    record(
        model,
        turn_id,
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        },
    );
}

#[test]
fn each_turn_s_message_knows_where_the_conversation_goes_back_to() {
    let mut model = model_with_thread();

    let first = run(&mut model, send("m1", "one")).unwrap();
    record(
        &mut model,
        "m1",
        ProviderOutput::SessionStarted {
            session_id: "s".into(),
        },
    );
    let steer = run(&mut model, send("m1b", "also")).unwrap();
    finish_turn(&mut model, "m1");
    let unknown = run(&mut model, send("m2", "two")).unwrap();
    finish_turn(&mut model, "m2");
    let cursor = record_conversation_cursor(&model, "t", "u2".into());
    model.apply(&cursor[0]);
    let second = run(&mut model, send("m3", "three")).unwrap();

    assert_eq!(rewind_of(&first), Some(Rewind::Start));
    assert_eq!(rewind_of(&steer), None);
    assert_eq!(rewind_of(&unknown), None);
    assert_eq!(
        rewind_of(&second),
        Some(Rewind::After {
            cursor: "u2".into(),
            session: Some("s".into()),
        })
    );
}

#[test]
fn a_rewind_pins_the_session_or_drops_it_and_a_new_cursor_unpins_it() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "one")).unwrap();
    record(
        &mut model,
        "m1",
        ProviderOutput::SessionStarted {
            session_id: "s".into(),
        },
    );
    let rewind = |rewind: Rewind| Command::ThreadRewind {
        thread_id: "t".into(),
        message_id: "m1".into(),
        rewind,
    };

    assert_eq!(
        run(&mut model, rewind(Rewind::Start)).unwrap_err(),
        CommandError::ThreadBusy {
            thread_id: "t".into()
        }
    );
    finish_turn(&mut model, "m1");
    run(
        &mut model,
        rewind(Rewind::After {
            cursor: "u1".into(),
            session: None,
        }),
    )
    .unwrap();
    let thread = model.thread("t").unwrap();
    assert_eq!(thread.session_id.as_deref(), Some("s"));
    assert_eq!(thread.resume_at.as_deref(), Some("u1"));
    let cursor = record_conversation_cursor(&model, "t", "u9".into());
    model.apply(&cursor[0]);
    assert_eq!(model.thread("t").unwrap().resume_at, None);

    run(&mut model, rewind(Rewind::Start)).unwrap();
    let thread = model.thread("t").unwrap();
    assert_eq!(thread.session_id, None);
    assert_eq!(thread.conversation_cursor, None);
}

#[test]
fn a_mention_without_text_or_image_is_empty() {
    let mut model = model_with_thread();
    let command = Command::MessageSend {
        thread_id: "t".into(),
        message_id: "m1".into(),
        text: String::new(),
        attachments: vec![MessageAttachment::Skill { name: "pdf".into() }],
    };

    assert_eq!(
        run(&mut model, command).unwrap_err(),
        CommandError::EmptyMessage
    );
}

#[test]
fn interrupting_an_idle_thread_fails() {
    let mut model = model_with_thread();

    let error = run(
        &mut model,
        Command::TurnInterrupt {
            thread_id: "t".into(),
        },
    )
    .unwrap_err();

    assert_eq!(
        error,
        CommandError::ThreadNotWorking {
            thread_id: "t".into()
        }
    );
}

#[test]
fn a_command_left_running_can_be_stopped_once_the_turn_ended_but_a_subagent_cannot() {
    let mut model = model_with_thread();
    let stop_command = Command::BackgroundTaskStop {
        thread_id: "t".into(),
        task_id: "b1".into(),
    };
    let stop_subagent = Command::SubagentStop {
        thread_id: "t".into(),
        tool_use_id: "toolu_1".into(),
    };

    assert_eq!(run(&mut model, stop_command), Ok(Vec::new()));
    assert!(run(&mut model, stop_subagent).is_err());
}

#[test]
fn the_agent_wakes_into_a_turn_of_its_own_only_on_an_idle_thread() {
    let mut model = model_with_thread();

    let woken = start_woken_turn(&model, "t", "w1", NOW);
    assert!(matches!(
        woken.as_slice(),
        [Event::TurnStarted { turn_id, .. }] if turn_id == "w1"
    ));
    apply_all(&mut model, &woken);
    assert!(start_woken_turn(&model, "t", "w2", NOW).is_empty());
    assert!(start_woken_turn(&model, "gone", "w3", NOW).is_empty());
}

#[test]
fn reasoning_is_kept_as_an_item_and_its_deltas_are_not() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "hi")).unwrap();

    let streamed = record(
        &mut model,
        "m1",
        ProviderOutput::ReasoningDelta {
            item_id: "r".into(),
            delta: "Let me".into(),
        },
    );
    let finished = record(
        &mut model,
        "m1",
        ProviderOutput::Reasoning {
            item_id: "r".into(),
            text: "Let me check.".into(),
        },
    );

    assert!(streamed.is_empty());
    assert!(matches!(
        &finished[..],
        [Event::ItemAdded { item }]
            if item.id == "r" && item.content == ItemContent::Reasoning { text: "Let me check.".into() }
    ));
}

#[test]
fn output_from_a_finished_turn_is_dropped() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "hi")).unwrap();
    record(
        &mut model,
        "m1",
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Interrupted,
        },
    );

    let late = record(
        &mut model,
        "m1",
        ProviderOutput::AssistantMessage {
            item_id: "a".into(),
            text: "too late".into(),
        },
    );

    assert!(late.is_empty());
    assert_eq!(model.thread("t").unwrap().status, ThreadStatus::Idle);
}

#[test]
fn a_failed_turn_records_the_reason_and_marks_the_thread() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "hi")).unwrap();

    let events = record(
        &mut model,
        "m1",
        turn_failed(TurnFailure::SpawnFailed {
            detail: "not found".into(),
        }),
    );

    assert!(matches!(
        &events[0],
        Event::ItemAdded { item } if matches!(item.content, ItemContent::TurnFailed { .. })
    ));
    let thread = model.thread("t").unwrap();
    assert_eq!(thread.status, ThreadStatus::Error);
    assert_eq!(thread.active_turn_id, None);
}

fn apply_all(model: &mut ReadModel, events: &[Event]) {
    for event in events {
        model.apply(event);
    }
}

#[test]
fn an_attached_worktree_becomes_the_thread_cwd() {
    let mut model = model_with_thread();

    let events = record_worktree(&model, "t", "/wt/fix".into(), Some("fix".into()), false);
    apply_all(&mut model, &events);

    assert_eq!(model.thread_cwd("t"), Some("/wt/fix"));
    assert_eq!(model.thread("t").unwrap().branch.as_deref(), Some("fix"));
    assert!(!model.thread("t").unwrap().worktree_adopted);
    assert!(record_worktree(&model, "t", "/wt/fix".into(), None, false).is_empty());
}

#[test]
fn an_adopted_worktree_is_never_removed_by_the_thread() {
    let mut model = model_with_thread();
    let events = record_worktree(&model, "t", "/wt/pr".into(), Some("feat/pr".into()), true);
    apply_all(&mut model, &events);

    assert!(model.thread("t").unwrap().worktree_adopted);
    for command in [
        Command::ThreadWorktreeRelease {
            thread_id: "t".into(),
        },
        Command::ThreadCleanUp {
            thread_id: "t".into(),
        },
    ] {
        assert_eq!(
            decide(&model, command, NOW).unwrap_err(),
            CommandError::WorktreeAdopted {
                thread_id: "t".into()
            }
        );
    }
}

#[test]
fn leaving_the_worktree_returns_the_thread_to_the_project_root() {
    let mut model = model_with_thread();
    let events = record_worktree(&model, "t", "/wt/pr".into(), Some("feat/pr".into()), true);
    apply_all(&mut model, &events);

    let events = record_worktree_left(&model, "t", Some("main".into()));
    apply_all(&mut model, &events);

    let thread = model.thread("t").unwrap();
    assert_eq!(model.thread_cwd("t"), Some("/repo"));
    assert_eq!(thread.branch.as_deref(), Some("main"));
    assert!(!thread.worktree_adopted);
    assert!(record_worktree_left(&model, "t", None).is_empty());
}

fn archive(archived: bool) -> Command {
    Command::ThreadArchive {
        thread_id: "t".into(),
        archived,
    }
}

fn pin(pinned: bool) -> Command {
    Command::ThreadPin {
        thread_id: "t".into(),
        pinned,
    }
}

#[test]
fn archiving_unpins_and_pinning_unarchives() {
    let mut model = model_with_thread();

    run(&mut model, pin(true)).unwrap();
    run(&mut model, archive(true)).unwrap();
    let thread = model.thread("t").unwrap();
    assert!(thread.archived && !thread.pinned);

    run(&mut model, pin(true)).unwrap();
    let thread = model.thread("t").unwrap();
    assert!(!thread.archived && thread.pinned);
}

#[test]
fn archiving_twice_changes_nothing() {
    let mut model = model_with_thread();
    run(&mut model, archive(true)).unwrap();

    assert!(run(&mut model, archive(true)).unwrap().is_empty());
}

#[test]
fn a_new_message_brings_an_archived_thread_back() {
    let mut model = model_with_thread();
    run(&mut model, archive(true)).unwrap();

    run(&mut model, send("m1", "one more thing")).unwrap();

    assert!(!model.thread("t").unwrap().archived);
}

#[test]
fn the_session_is_kept_for_the_next_turn() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "hi")).unwrap();

    record(
        &mut model,
        "m1",
        ProviderOutput::SessionStarted {
            session_id: "s1".into(),
        },
    );

    assert_eq!(model.thread("t").unwrap().session_id.as_deref(), Some("s1"));
}

#[test]
fn a_worktree_thread_runs_in_its_worktree_and_a_local_one_in_the_root() {
    let mut model = model_with_thread();
    let mut command = create_thread("w", "p");
    if let Command::ThreadCreate { workspace, .. } = &mut command {
        *workspace = ThreadWorkspace::Worktree {
            path: "/worktrees/w".into(),
            branch: "hoshi/w".into(),
        };
    }

    run(&mut model, command).unwrap();

    assert_eq!(model.thread_cwd("t"), Some("/repo"));
    assert_eq!(model.thread_cwd("w"), Some("/worktrees/w"));
    assert_eq!(
        model.thread("w").unwrap().branch.as_deref(),
        Some("hoshi/w")
    );
}

#[test]
fn an_unprepared_worktree_is_never_created() {
    let mut model = model_with_thread();
    let mut command = create_thread("w", "p");
    if let Command::ThreadCreate { workspace, .. } = &mut command {
        *workspace = ThreadWorkspace::NewWorktree;
    }

    let error = run(&mut model, command).unwrap_err();

    assert!(matches!(error, CommandError::Internal { .. }));
    assert!(model.thread("w").is_none());
}

#[test]
fn a_branch_is_recorded_only_when_it_changes() {
    let mut model = model_with_thread();

    let first = record_branch(&model, "t", Some("main".into()));
    first.iter().for_each(|event| model.apply(event));
    let again = record_branch(&model, "t", Some("main".into()));

    assert_eq!(first.len(), 1);
    assert!(again.is_empty());
    assert_eq!(model.thread("t").unwrap().branch.as_deref(), Some("main"));
}

#[test]
fn the_model_choice_is_kept_on_the_thread() {
    let mut model = model_with_thread();

    run(
        &mut model,
        Command::ThreadModelSet {
            thread_id: "t".into(),
            provider: crate::default_provider(),
            model: Some("opus".into()),
            effort: Some(Effort::High),
            fast_mode: true,
            ultracode: true,
        },
    )
    .unwrap();

    let thread = model.thread("t").unwrap();
    assert_eq!(thread.model.as_deref(), Some("opus"));
    assert_eq!(thread.effort, Some(Effort::High));
    assert!(thread.fast_mode && thread.ultracode);
}

fn switch_to_copilot() -> Command {
    Command::ThreadModelSet {
        thread_id: "t".into(),
        provider: "copilot".into(),
        model: Some("gpt-6.1-sol".into()),
        effort: None,
        fast_mode: false,
        ultracode: false,
    }
}

#[test]
fn a_thread_without_a_session_can_move_to_another_provider() {
    let mut model = model_with_thread();

    run(&mut model, switch_to_copilot()).unwrap();

    assert_eq!(model.thread("t").unwrap().provider, "copilot");
}

fn switch_to(provider: &str) -> Command {
    Command::ThreadModelSet {
        thread_id: "t".into(),
        provider: provider.into(),
        model: None,
        effort: None,
        fast_mode: false,
        ultracode: false,
    }
}

/// The thread ran turn `m1` on its first provider, in session `s`, which ended at `c1`.
fn model_with_conversation() -> ReadModel {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "one")).unwrap();
    record(
        &mut model,
        "m1",
        ProviderOutput::SessionStarted {
            session_id: "s".into(),
        },
    );
    finish_turn(&mut model, "m1");
    let cursor = record_conversation_cursor(&model, "t", "c1".into());
    model.apply(&cursor[0]);
    model
}

#[test]
fn a_provider_picked_once_the_thread_has_a_session_waits_for_the_next_message() {
    let mut model = model_with_conversation();

    run(&mut model, switch_to_copilot()).unwrap();
    let thread = model.thread("t").unwrap();
    assert_eq!(thread.provider, crate::DEFAULT_PROVIDER);
    assert_eq!(thread.pending_model.as_ref().unwrap().provider, "copilot");

    let sent = run(&mut model, send("m2", "two")).unwrap();

    let thread = model.thread("t").unwrap();
    assert_eq!(thread.provider, "copilot");
    assert_eq!(thread.model.as_deref(), Some("gpt-6.1-sol"));
    assert_eq!(thread.pending_model, None);
    assert_eq!(thread.session_id, None);
    assert_eq!(thread.conversation_cursor, None);
    assert_eq!(
        thread.provider_sessions,
        [ProviderSession {
            provider: crate::DEFAULT_PROVIDER.into(),
            session_id: "s".into(),
            conversation_cursor: Some("c1".into()),
            resume_at: None,
            seen_before: "m2".into(),
        }]
    );
    assert!(matches!(
        thread.pending_handoffs.as_slice(),
        [PendingHandoff::ProviderSwitch {
            strategy: HandoffStrategy::FullThreadSummary,
            since: None,
            before,
            from,
            to,
        }] if before == "m2" && from.provider == crate::DEFAULT_PROVIDER && to.provider == "copilot"
    ));
    assert_eq!(rewind_of(&sent), None);
}

#[test]
fn switching_back_resumes_the_provider_s_session_and_hands_it_only_what_it_missed() {
    let mut model = model_with_conversation();
    run(&mut model, switch_to_copilot()).unwrap();
    run(&mut model, send("m2", "two")).unwrap();
    record(
        &mut model,
        "m2",
        ProviderOutput::SessionStarted {
            session_id: "x".into(),
        },
    );
    finish_turn(&mut model, "m2");

    run(&mut model, switch_to(crate::DEFAULT_PROVIDER)).unwrap();
    run(&mut model, send("m3", "three")).unwrap();

    let thread = model.thread("t").unwrap();
    assert_eq!(thread.provider, crate::DEFAULT_PROVIDER);
    assert_eq!(thread.session_id.as_deref(), Some("s"));
    assert_eq!(thread.conversation_cursor.as_deref(), Some("c1"));
    assert_eq!(thread.provider_sessions.len(), 1);
    assert_eq!(thread.provider_sessions[0].provider, "copilot");
    assert_eq!(thread.provider_sessions[0].seen_before, "m3");
    assert!(matches!(
        thread.pending_handoffs.as_slice(),
        [PendingHandoff::ProviderSwitch {
            strategy: HandoffStrategy::DeltaSinceTargetLastSeen,
            since: Some(since),
            before,
            ..
        }] if since == "m2" && before == "m3"
    ));
}

#[test]
fn another_provider_picked_while_a_turn_runs_waits_for_the_next_turn() {
    let mut model = model_with_conversation();
    run(&mut model, send("m2", "two")).unwrap();

    run(&mut model, switch_to_copilot()).unwrap();
    run(&mut model, send("steer", "and this")).unwrap();
    let thread = model.thread("t").unwrap();
    assert_eq!(thread.provider, crate::DEFAULT_PROVIDER);
    assert_eq!(thread.active_turn_id.as_deref(), Some("m2"));
    assert_eq!(thread.pending_model.as_ref().unwrap().provider, "copilot");

    finish_turn(&mut model, "m2");
    run(&mut model, send("m3", "three")).unwrap();

    let thread = model.thread("t").unwrap();
    assert_eq!(thread.provider, "copilot");
    assert_eq!(thread.pending_model, None);
    assert_eq!(thread.provider_sessions[0].seen_before, "m3");
}

#[test]
fn only_the_users_message_moves_the_thread_to_the_provider_they_picked() {
    let mut model = model_with_conversation();
    run(&mut model, switch_to_copilot()).unwrap();
    let turns: [fn(&ReadModel) -> Vec<Event>; 4] = [
        |model| start_queued_turn(model, "t", "x1", None, NOW),
        |model| start_resumed_turn(model, "t", "x2", "go on", Resumption::Restart, NOW),
        |model| send_task_result(model, "t", "child", "x3", "done", NOW).unwrap(),
        |model| {
            let update = crate::PullRequestUpdate::default();
            send_pull_request_update(model, "t", "x4", "news", update, NOW).unwrap()
        },
    ];
    for (index, start) in turns.into_iter().enumerate() {
        for event in start(&model) {
            model.apply(&event);
        }
        let turn = format!("x{}", index + 1);
        let thread = model.thread("t").unwrap();
        assert_eq!(thread.active_turn_id.as_deref(), Some(turn.as_str()));
        assert_eq!(thread.provider, crate::DEFAULT_PROVIDER);
        assert_eq!(thread.pending_model.as_ref().unwrap().provider, "copilot");
        finish_turn(&mut model, &turn);
    }
    run(&mut model, send("m2", "/compact")).unwrap();
    assert_eq!(model.thread("t").unwrap().provider, crate::DEFAULT_PROVIDER);
    finish_turn(&mut model, "m2");

    run(&mut model, send("m3", "over there")).unwrap();
    assert_eq!(model.thread("t").unwrap().provider, "copilot");
}

#[test]
fn the_current_provider_drops_the_pick() {
    let mut model = model_with_conversation();

    run(&mut model, switch_to_copilot()).unwrap();
    run(&mut model, switch_to(crate::DEFAULT_PROVIDER)).unwrap();
    run(&mut model, send("m2", "two")).unwrap();

    let thread = model.thread("t").unwrap();
    assert_eq!(thread.provider, crate::DEFAULT_PROVIDER);
    assert_eq!(thread.session_id.as_deref(), Some("s"));
    assert!(thread.pending_handoffs.is_empty());
}

#[test]
fn another_provider_picked_while_a_first_turn_runs_without_a_session_waits_too() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "one")).unwrap();

    run(&mut model, switch_to_copilot()).unwrap();
    assert_eq!(
        model.thread("t").unwrap().provider,
        crate::DEFAULT_PROVIDER,
        "the turn goes on where it started"
    );
    finish_turn(&mut model, "m1");
    run(&mut model, switch_to("copilot")).unwrap();
    assert!(
        model.thread("t").unwrap().pending_model.is_some(),
        "picked again, it still waits"
    );
    run(&mut model, send("m2", "two")).unwrap();

    let thread = model.thread("t").unwrap();
    assert_eq!(thread.provider, "copilot");
    assert_eq!(thread.pending_handoffs.len(), 1);
}

fn model_switch_of(events: &[Event]) -> Option<(Option<Effort>, Option<Effort>)> {
    events.iter().find_map(|event| match event {
        Event::ItemAdded {
            item:
                ThreadItem {
                    content: ItemContent::UserMessage { model_switch, .. },
                    ..
                },
        } => Some(
            model_switch
                .as_deref()
                .map(|switch| (switch.from.effort, switch.to.effort)),
        ),
        _ => None,
    })?
}

#[test]
fn a_turn_on_another_effort_or_model_of_the_provider_says_what_changed() {
    let mut model = model_with_conversation();
    let set = |provider: &str, slug: &str, effort| Command::ThreadModelSet {
        thread_id: "t".into(),
        provider: provider.into(),
        model: Some(slug.into()),
        effort,
        fast_mode: false,
        ultracode: false,
    };
    run(
        &mut model,
        set(crate::DEFAULT_PROVIDER, "opus", Some(Effort::Medium)),
    )
    .unwrap();
    let first = run(&mut model, send("m2", "two")).unwrap();
    assert_eq!(model_switch_of(&first), Some((None, Some(Effort::Medium))));

    let steer = run(&mut model, send("m2b", "also")).unwrap();
    assert_eq!(model_switch_of(&steer), None);
    finish_turn(&mut model, "m2");

    let same = run(&mut model, send("m3", "three")).unwrap();
    assert_eq!(model_switch_of(&same), None);
    finish_turn(&mut model, "m3");

    run(
        &mut model,
        set(crate::DEFAULT_PROVIDER, "opus", Some(Effort::High)),
    )
    .unwrap();
    let higher = run(&mut model, send("m4", "four")).unwrap();
    assert_eq!(
        model_switch_of(&higher),
        Some((Some(Effort::Medium), Some(Effort::High)))
    );
    finish_turn(&mut model, "m4");

    run(&mut model, switch_to_copilot()).unwrap();
    let other_provider = run(&mut model, send("m5", "five")).unwrap();
    assert_eq!(model_switch_of(&other_provider), None);
}

#[test]
fn only_a_turn_the_user_s_message_opens_records_what_it_runs_on() {
    let mut model = model_with_conversation();
    let apply = |model: &mut ReadModel, events: &[Event]| {
        for event in events {
            model.apply(event);
        }
    };
    let last = |model: &ReadModel| model.threads["t"].last_turn_model.clone();
    run(
        &mut model,
        Command::ThreadModelSet {
            thread_id: "t".into(),
            provider: crate::DEFAULT_PROVIDER.into(),
            model: Some("opus".into()),
            effort: Some(Effort::High),
            fast_mode: false,
            ultracode: false,
        },
    )
    .unwrap();
    let before = last(&model);

    let compact = run(&mut model, send("c1", "/compact")).unwrap();
    assert_eq!(model_switch_of(&compact), None);
    finish_turn(&mut model, "c1");
    let resumed = start_resumed_turn(&model, "t", "r1", "go on", Resumption::Restart, NOW);
    apply(&mut model, &resumed);
    assert_eq!(model_switch_of(&resumed), None);
    finish_turn(&mut model, "r1");
    let woken = start_woken_turn(&model, "t", "w1", NOW);
    apply(&mut model, &woken);
    finish_turn(&mut model, "w1");
    assert_eq!(last(&model), before, "none of them is the user's message");

    let shown = run(&mut model, send("m2", "two")).unwrap();
    assert_eq!(model_switch_of(&shown), Some((None, Some(Effort::High))));
    finish_turn(&mut model, "m2");

    run(
        &mut model,
        Command::ThreadRewind {
            thread_id: "t".into(),
            message_id: "m2".into(),
            rewind: rewind_of(&shown).unwrap(),
        },
    )
    .unwrap();
    assert_eq!(last(&model), None);
    let again = run(&mut model, send("m3", "two again")).unwrap();
    assert_eq!(model_switch_of(&again), None);
    finish_turn(&mut model, "m3");

    run(&mut model, switch_to_copilot()).unwrap();
    run(&mut model, send("q1", "over there")).unwrap();
    assert_eq!(last(&model).unwrap().provider, "copilot");
    finish_turn(&mut model, "q1");

    run(
        &mut model,
        Command::ThreadModelSet {
            thread_id: "t".into(),
            provider: "copilot".into(),
            model: Some("gpt-6.1-sol".into()),
            effort: Some(Effort::High),
            fast_mode: false,
            ultracode: false,
        },
    )
    .unwrap();
    let kept = ThreadItem {
        id: "q2".into(),
        thread_id: "t".into(),
        created_at: NOW.into(),
        content: ItemContent::UserMessage {
            text: "sent late".into(),
            attachments: Vec::new(),
            steer: true,
            task_result: None,
            pull_request_update: None,
            resumed: None,
            rewind: None,
            dropped: false,
            model_switch: None,
        },
    };
    let late = start_queued_turn(&model, "t", "q2", Some(&kept), NOW);
    let marked = late.iter().find_map(|event| match event {
        Event::ItemAdded {
            item:
                ThreadItem {
                    id,
                    content: ItemContent::ModelSwitched { from, to },
                    ..
                },
        } => Some((id.as_str(), from.effort, to.effort)),
        _ => None,
    });
    assert_eq!(marked, Some(("q2:model", None, Some(Effort::High))));
    assert!(matches!(late.last(), Some(Event::TurnStarted { .. })));
}

#[test]
fn a_message_that_moves_provider_shows_the_handoff_alone() {
    let mut model = model_with_conversation();
    let set = |provider: &str, slug: &str, effort| Command::ThreadModelSet {
        thread_id: "t".into(),
        provider: provider.into(),
        model: Some(slug.into()),
        effort,
        fast_mode: false,
        ultracode: false,
    };
    run(
        &mut model,
        set(crate::DEFAULT_PROVIDER, "opus", Some(Effort::Medium)),
    )
    .unwrap();
    run(&mut model, send("m2", "two")).unwrap();
    finish_turn(&mut model, "m2");
    run(&mut model, switch_to_copilot()).unwrap();
    run(&mut model, send("r1", "over there")).unwrap();
    finish_turn(&mut model, "r1");
    assert_eq!(model.threads["t"].provider, "copilot");

    run(
        &mut model,
        set(crate::DEFAULT_PROVIDER, "opus", Some(Effort::High)),
    )
    .unwrap();
    let back = run(&mut model, send("m3", "back")).unwrap();
    assert!(
        back.iter()
            .any(|event| matches!(event, Event::ProviderSwitched { .. }))
    );
    assert_eq!(model_switch_of(&back), None);
}

#[test]
fn a_rewind_to_before_a_provider_switch_is_refused() {
    let mut model = model_with_conversation();
    let second = run(&mut model, send("m2", "two")).unwrap();
    finish_turn(&mut model, "m2");
    run(&mut model, switch_to_copilot()).unwrap();
    run(&mut model, send("m3", "three")).unwrap();
    finish_turn(&mut model, "m3");

    let error = run(
        &mut model,
        Command::ThreadRewind {
            thread_id: "t".into(),
            message_id: "m2".into(),
            rewind: rewind_of(&second).unwrap(),
        },
    )
    .unwrap_err();

    assert!(matches!(error, CommandError::CannotRewind { .. }));
}

fn turn_point(turn_id: &str, status: TurnStatus) -> TurnPoint {
    TurnPoint {
        turn_id: turn_id.into(),
        through_item_id: format!("{turn_id}:answer"),
        ordinal: 1,
        status,
        provider: crate::DEFAULT_PROVIDER.into(),
        session_id: Some("s".into()),
        cursor: Some("c1".into()),
    }
}

fn fork(fork_thread_id: &str, status: TurnStatus) -> Command {
    Command::ThreadFork {
        thread_id: "t".into(),
        fork_thread_id: fork_thread_id.into(),
        turn: turn_point("m1", status),
    }
}

#[test]
fn a_fork_starts_beside_its_source_with_none_of_its_place_in_the_list() {
    let mut model = model_with_conversation();
    let source = model.threads.get_mut("t").unwrap();
    source.worktree_path = Some("/repo/wt".into());
    source.branch = Some("hoshi/t".into());
    source.pinned = true;
    source.watching_pull_request = true;
    source.title = "Fix login".into();

    run(&mut model, fork("f", TurnStatus::Completed)).unwrap();

    let fork = model.thread("f").unwrap();
    assert_eq!(fork.title, "Fix login fork");
    assert_eq!(fork.project_id.as_deref(), Some("p"));
    assert_eq!(fork.worktree_path.as_deref(), Some("/repo/wt"));
    assert_eq!(fork.branch.as_deref(), Some("hoshi/t"));
    assert!(!fork.pinned && !fork.watching_pull_request && !fork.archived);
    assert_eq!(fork.session_id, None);
    assert_eq!(fork.forked_from.as_ref().unwrap().thread_id, "t");
    assert_eq!(fork.forked_from.as_ref().unwrap().turn.turn_id, "m1");
    assert_eq!(
        fork.pending_handoffs,
        [PendingHandoff::Fork { native: true }]
    );
    assert_eq!(model.forks_of("t").len(), 1);
}

#[test]
fn a_turn_still_running_cannot_be_forked_but_one_that_failed_can() {
    let mut model = model_with_conversation();

    assert_eq!(
        run(&mut model, fork("f", TurnStatus::Running)).unwrap_err(),
        CommandError::TurnNotForkable {
            thread_id: "t".into(),
            turn_id: "m1".into()
        }
    );
    run(&mut model, fork("f", TurnStatus::Failed)).unwrap();
    run(&mut model, fork("g", TurnStatus::Interrupted)).unwrap();
}

#[test]
fn a_fork_rewound_to_its_start_takes_its_source_again() {
    let mut model = model_with_conversation();
    run(&mut model, fork("f", TurnStatus::Completed)).unwrap();
    model.threads.get_mut("f").unwrap().pending_handoffs.clear();

    run(
        &mut model,
        Command::ThreadRewind {
            thread_id: "f".into(),
            message_id: "f1".into(),
            rewind: Rewind::Start,
        },
    )
    .unwrap();

    assert_eq!(
        model.thread("f").unwrap().pending_handoffs,
        [PendingHandoff::Fork { native: true }]
    );
}

fn merge_back(fork_thread_id: &str, status: TurnStatus) -> Command {
    Command::ThreadMergeBack {
        thread_id: fork_thread_id.into(),
        turn: turn_point(&format!("{fork_thread_id}1"), status),
    }
}

#[test]
fn a_merge_back_waits_for_the_parent_s_next_turn_and_replaces_an_older_one() {
    let mut model = model_with_conversation();
    run(&mut model, fork("f", TurnStatus::Completed)).unwrap();

    run(&mut model, merge_back("f", TurnStatus::Completed)).unwrap();
    let mut newer = merge_back("f", TurnStatus::Completed);
    if let Command::ThreadMergeBack { turn, .. } = &mut newer {
        turn.ordinal = 2;
    }
    run(&mut model, newer).unwrap();

    let handoffs = &model.thread("t").unwrap().pending_handoffs;
    assert!(matches!(
        handoffs.as_slice(),
        [PendingHandoff::MergeBack { fork_thread_id, turn }]
            if fork_thread_id == "f" && turn.ordinal == 2
    ));
    let delivered = handoffs.clone();
    let events = consume_handoffs(&model, "t", &delivered);
    events.iter().for_each(|event| model.apply(event));
    assert!(model.thread("t").unwrap().pending_handoffs.is_empty());
}

#[test]
fn a_merge_back_is_refused_from_no_fork_an_unfinished_turn_a_busy_parent_or_a_rival() {
    let mut model = model_with_conversation();
    run(&mut model, fork("f", TurnStatus::Completed)).unwrap();
    run(&mut model, fork("g", TurnStatus::Completed)).unwrap();

    assert_eq!(
        run(
            &mut model,
            Command::ThreadMergeBack {
                thread_id: "t".into(),
                turn: turn_point("m1", TurnStatus::Completed),
            }
        )
        .unwrap_err(),
        CommandError::NotAFork {
            thread_id: "t".into()
        }
    );
    assert_eq!(
        run(&mut model, merge_back("f", TurnStatus::Failed)).unwrap_err(),
        CommandError::MergeBackTurnNotFinished {
            thread_id: "f".into(),
            turn_id: "f1".into()
        }
    );
    run(&mut model, send("m2", "two")).unwrap();
    assert_eq!(
        run(&mut model, merge_back("f", TurnStatus::Completed)).unwrap_err(),
        CommandError::MergeBackTargetBusy {
            thread_id: "t".into()
        }
    );
    finish_turn(&mut model, "m2");
    run(&mut model, merge_back("f", TurnStatus::Completed)).unwrap();
    assert_eq!(
        run(&mut model, merge_back("g", TurnStatus::Completed)).unwrap_err(),
        CommandError::MergeBackPending {
            thread_id: "t".into(),
            fork_thread_id: "f".into()
        }
    );
}

#[test]
fn the_access_choice_is_kept_on_the_thread() {
    let mut model = model_with_thread();
    assert_eq!(
        model.thread("t").unwrap().access_mode,
        AccessMode::AcceptEdits
    );

    run(
        &mut model,
        Command::ThreadAccessSet {
            thread_id: "t".into(),
            access_mode: AccessMode::FullAccess,
            plan_mode: true,
        },
    )
    .unwrap();

    let thread = model.thread("t").unwrap();
    assert_eq!(thread.access_mode, AccessMode::FullAccess);
    assert!(thread.plan_mode);
}

fn approval() -> ItemContent {
    ItemContent::Approval {
        tool_call_id: Some("toolu_1".into()),
        tool_name: "Bash".into(),
        input: r#"{"command":"npm test"}"#.into(),
        reason: None,
        can_accept_for_session: false,
        status: RequestStatus::Pending,
        decision: None,
        feedback: None,
    }
}

fn open(model: &mut ReadModel, request_id: &str, content: ItemContent) -> Vec<Event> {
    record(
        model,
        "m1",
        ProviderOutput::RequestOpened {
            request_id: request_id.into(),
            content,
        },
    )
}

fn respond(request_id: &str, response: RequestResponse) -> Command {
    Command::RequestRespond {
        thread_id: "t".into(),
        request_id: request_id.into(),
        response,
    }
}

fn decision(decision: ApprovalDecision) -> RequestResponse {
    RequestResponse::Decision {
        decision,
        feedback: None,
    }
}

fn status(model: &ReadModel) -> ThreadStatus {
    model.thread("t").unwrap().status
}

#[test]
fn a_request_waits_for_the_user_until_answered() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "test it")).unwrap();
    open(&mut model, "r1", approval());
    assert_eq!(status(&model), ThreadStatus::WaitingForInput);
    assert_eq!(model.pending_requests("t").len(), 1);

    let events = run(
        &mut model,
        respond(
            "r1",
            RequestResponse::Decision {
                decision: ApprovalDecision::Decline,
                feedback: Some("  run the unit tests only ".into()),
            },
        ),
    )
    .unwrap();

    let Event::ItemAdded { item } = &events[0] else {
        panic!("expected the resolved request, got {events:?}");
    };
    assert!(matches!(
        &item.content,
        ItemContent::Approval {
            status: RequestStatus::Resolved,
            decision: Some(ApprovalDecision::Decline),
            feedback: Some(feedback),
            ..
        } if feedback == "run the unit tests only"
    ));
    assert_eq!(status(&model), ThreadStatus::Working);
    assert!(model.pending_requests("t").is_empty());
}

#[test]
fn the_thread_waits_while_any_request_is_open() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "test it")).unwrap();
    open(&mut model, "r1", approval());
    let again = open(&mut model, "r2", approval());
    assert_eq!(again.len(), 1, "already waiting: {again:?}");

    run(
        &mut model,
        respond("r1", decision(ApprovalDecision::Accept)),
    )
    .unwrap();
    assert_eq!(status(&model), ThreadStatus::WaitingForInput);

    record(
        &mut model,
        "m1",
        ProviderOutput::RequestWithdrawn {
            request_id: "r2".into(),
        },
    );
    assert_eq!(status(&model), ThreadStatus::Working);
}

#[test]
fn a_request_is_answered_once_and_only_as_it_asks() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "test it")).unwrap();
    open(&mut model, "r1", approval());

    let answers = RequestResponse::Answers {
        answers: vec!["yes".into()],
    };
    assert_eq!(
        run(&mut model, respond("r1", answers)),
        Err(CommandError::RequestResponseInvalid {
            request_id: "r1".into()
        })
    );
    assert_eq!(
        run(
            &mut model,
            respond("r1", decision(ApprovalDecision::AcceptForSession))
        ),
        Err(CommandError::RequestResponseInvalid {
            request_id: "r1".into()
        }),
        "this approval offers no session grant"
    );
    run(
        &mut model,
        respond("r1", decision(ApprovalDecision::Accept)),
    )
    .unwrap();
    assert_eq!(
        run(
            &mut model,
            respond("r1", decision(ApprovalDecision::Accept))
        ),
        Err(CommandError::RequestNotPending {
            request_id: "r1".into()
        })
    );
}

#[test]
fn questions_take_one_answer_each() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "ask me")).unwrap();
    let question = Question {
        header: "Color".into(),
        question: "Which color?".into(),
        options: Vec::new(),
        multi_select: false,
    };
    open(
        &mut model,
        "r1",
        ItemContent::Questions {
            questions: vec![question.clone(), question],
            status: RequestStatus::Pending,
            answers: None,
        },
    );

    let one = RequestResponse::Answers {
        answers: vec!["Red".into()],
    };
    assert!(run(&mut model, respond("r1", one)).is_err());
    let both = RequestResponse::Answers {
        answers: vec!["Red".into(), "Blue".into()],
    };
    let events = run(&mut model, respond("r1", both)).unwrap();
    assert!(matches!(
        &events[0],
        Event::ItemAdded { item } if matches!(
            &item.content,
            ItemContent::Questions { answers: Some(answers), status: RequestStatus::Resolved, .. }
                if answers.len() == 2
        )
    ));
}

#[test]
fn an_accepted_plan_takes_the_thread_out_of_plan_mode() {
    let mut model = model_with_thread();
    run(
        &mut model,
        Command::ThreadAccessSet {
            thread_id: "t".into(),
            access_mode: AccessMode::Ask,
            plan_mode: true,
        },
    )
    .unwrap();
    run(&mut model, send("m1", "plan it")).unwrap();
    open(
        &mut model,
        "r1",
        ItemContent::ProposedPlan {
            plan: "1. Do it".into(),
            status: RequestStatus::Pending,
            decision: None,
            feedback: None,
        },
    );

    run(
        &mut model,
        respond("r1", decision(ApprovalDecision::Accept)),
    )
    .unwrap();

    let thread = model.thread("t").unwrap();
    assert!(!thread.plan_mode);
    assert_eq!(thread.access_mode, AccessMode::Ask);
    assert_eq!(thread.status, ThreadStatus::Working);
}

#[test]
fn requests_left_open_are_cancelled_when_the_turn_ends() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "test it")).unwrap();
    open(&mut model, "r1", approval());

    let events = record(
        &mut model,
        "m1",
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Interrupted,
        },
    );

    assert!(matches!(
        &events[0],
        Event::ItemAdded { item } if item.content.request_status() == Some(RequestStatus::Cancelled)
    ));
    assert!(model.pending_requests("t").is_empty());
    assert_eq!(status(&model), ThreadStatus::Idle);
    assert_eq!(
        run(
            &mut model,
            respond("r1", decision(ApprovalDecision::Accept))
        ),
        Err(CommandError::RequestNotPending {
            request_id: "r1".into()
        })
    );
}

#[test]
fn a_turn_the_app_cut_short_keeps_its_questions_for_an_answer_by_message() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "test it")).unwrap();
    open(&mut model, "r1", approval());
    let asked = ItemContent::Questions {
        questions: vec![Question {
            header: "Color".into(),
            question: "Which color?".into(),
            options: Vec::new(),
            multi_select: false,
        }],
        status: RequestStatus::Pending,
        answers: None,
    };
    open(&mut model, "r2", asked.clone());

    let events = close_stale_turn(&model, "t", "m1", NOW);
    for event in &events {
        model.apply(event);
    }

    let pending: Vec<_> = model
        .pending_requests("t")
        .iter()
        .map(|item| item.id.clone())
        .collect();
    assert_eq!(pending, ["r2"]);
    assert_eq!(status(&model), ThreadStatus::Idle);
    let answers = RequestResponse::Answers {
        answers: vec![" Blue ".into()],
    };
    assert_eq!(
        answers_message(&asked, &answers).as_deref(),
        Some(
            "Which color?
Blue"
        )
    );
}

#[test]
fn a_turn_the_app_cut_short_reads_as_cut_not_as_stopped_by_the_user() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "test it")).unwrap();

    let ends: Vec<_> = close_stale_turn(&model, "t", "m1", NOW)
        .into_iter()
        .filter_map(|event| match event {
            Event::ItemAdded { item } => Some(item.content),
            _ => None,
        })
        .filter(|content| matches!(content, ItemContent::TurnCut | ItemContent::TurnInterrupted))
        .collect();

    assert_eq!(ends, [ItemContent::TurnCut]);
    let old: ItemContent = serde_json::from_str(r#"{"kind":"turnInterrupted"}"#).unwrap();
    assert_eq!(old, ItemContent::TurnInterrupted);
    let cut = serde_json::to_string(&ItemContent::TurnCut).unwrap();
    assert_eq!(cut, r#"{"kind":"turnCut"}"#);
}

#[test]
fn a_rewind_drops_the_questions_it_takes_out_of_the_transcript() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "test it")).unwrap();
    let asked = ItemContent::Questions {
        questions: vec![Question {
            header: "Color".into(),
            question: "Which color?".into(),
            options: Vec::new(),
            multi_select: false,
        }],
        status: RequestStatus::Pending,
        answers: None,
    };
    open(&mut model, "r1", asked);
    for event in close_stale_turn(&model, "t", "m1", NOW) {
        model.apply(&event);
    }
    assert_eq!(model.pending_requests("t").len(), 1);

    run(
        &mut model,
        Command::ThreadRewind {
            thread_id: "t".into(),
            message_id: "m1".into(),
            rewind: Rewind::Start,
        },
    )
    .unwrap();

    assert!(model.pending_requests("t").is_empty());
    assert!(model.pending_request("r1").is_none());
    let answers = RequestResponse::Answers {
        answers: vec!["Blue".into()],
    };
    assert!(run(&mut model, respond("r1", answers)).is_err());
}

#[test]
fn a_page_keeps_only_the_start_of_long_tool_output() {
    let call = |output: &str| ItemContent::ToolCall {
        name: "Bash".into(),
        input: "{}".into(),
        output: Some(output.into()),
        is_error: false,
        detail: None,
        output_omitted: false,
        interrupted: false,
    };
    let long = "x".repeat(1_000);

    let ItemContent::ToolCall {
        output,
        output_omitted,
        ..
    } = call(&long).summary()
    else {
        unreachable!()
    };
    assert_eq!(output.map(|text| text.len()), Some(200));
    assert!(output_omitted);
    assert_eq!(call("short").summary(), call("short"));
}

#[test]
fn a_subagent_writes_its_own_transcript_beside_the_thread() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "count the files")).unwrap();
    let nested = |tool_use_id: &str, output| ProviderOutput::Subagent {
        tool_use_id: tool_use_id.into(),
        output: Box::new(output),
    };

    let started = record(
        &mut model,
        "m1",
        nested(
            "toolu_agent",
            ProviderOutput::ToolCallStarted {
                item_id: "toolu_ls".into(),
                name: "Bash".into(),
                input: "{}".into(),
            },
        ),
    );
    let ignored = record(
        &mut model,
        "m1",
        nested(
            "toolu_agent",
            ProviderOutput::TextDelta {
                item_id: "x".into(),
                delta: "hi".into(),
            },
        ),
    );

    assert!(matches!(
        started.as_slice(),
        [Event::ItemAdded { item }] if item.thread_id == "t:toolu_agent" && item.id == "toolu_ls"
    ));
    assert!(ignored.is_empty());
    assert_eq!(
        subagent_thread_id("t:toolu_agent", "toolu_inner"),
        "t:toolu_inner"
    );
    assert_eq!(model.thread("t").unwrap().status, ThreadStatus::Working);
}

#[test]
fn calls_left_running_stop_with_their_turn_subagents_included() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "run it")).unwrap();
    let start = |item_id: &str| ProviderOutput::ToolCallStarted {
        item_id: item_id.into(),
        name: "Bash".into(),
        input: "{}".into(),
    };
    record(&mut model, "m1", start("done"));
    record(&mut model, "m1", start("long"));
    record(
        &mut model,
        "m1",
        ProviderOutput::Subagent {
            tool_use_id: "toolu_agent".into(),
            output: Box::new(start("inner")),
        },
    );
    record(
        &mut model,
        "m1",
        ProviderOutput::ToolCallCompleted {
            item_id: "done".into(),
            output: "ok".into(),
            is_error: false,
            detail: None,
        },
    );

    let events = record(
        &mut model,
        "m1",
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Interrupted,
        },
    );

    let interrupted: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            Event::ToolCallInterrupted { thread_id, item_id } => {
                Some(format!("{thread_id}/{item_id}"))
            }
            _ => None,
        })
        .collect();
    assert_eq!(interrupted, ["t/long", "t:toolu_agent/inner"]);
    assert_eq!(model.open_tool_calls().count(), 0);
}

#[test]
fn a_delegated_thread_knows_its_parent_and_gets_its_answer_back_as_hoshi() {
    let mut model = model_with_thread();
    run(&mut model, create_thread("c", "p")).unwrap();
    for event in record_delegation(&model, "c", "t") {
        model.apply(&event);
    }
    assert_eq!(
        model.thread("c").unwrap().parent_thread_id.as_deref(),
        Some("t")
    );
    assert!(record_delegation(&model, "c", "missing").is_empty());

    let events = send_task_result(&model, "t", "c", "r1", "done", NOW).unwrap();

    assert!(events.iter().any(|event| matches!(
        event,
        Event::ItemAdded { item } if matches!(
            &item.content,
            ItemContent::UserMessage { task_result: Some(child), .. } if child == "c"
        )
    )));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::TurnStarted { turn_id, .. } if turn_id == "r1"))
    );
}

#[cfg(windows)]
#[test]
fn windows_paths_compare_case_and_separator_insensitively() {
    assert!(same_path(r"C:\Repo\Hoshi", "c:/repo/hoshi/"));
}

fn set_setup(files: &[&str], command: &str) -> Command {
    Command::ProjectWorktreeSetupSet {
        project_id: "p".into(),
        setup: Some(WorktreeSetup {
            files: files.iter().map(|&file| file.to_owned()).collect(),
            command: command.into(),
            wait: true,
        }),
    }
}

fn setup_of(model: &ReadModel) -> Option<WorktreeSetup> {
    model.project("p").and_then(|p| p.worktree_setup.clone())
}

#[test]
fn a_worktree_setup_keeps_its_files_relative_and_once_each() {
    let mut model = ReadModel::default();
    run(&mut model, create_project("p", "/repo")).unwrap();

    run(
        &mut model,
        set_setup(
            &[
                " .env ",
                "",
                r"config\local.properties",
                "./.env",
                "secrets//*.env/",
            ],
            "  ",
        ),
    )
    .unwrap();

    let setup = setup_of(&model).unwrap();
    assert_eq!(
        setup.files,
        [".env", "config/local.properties", "secrets/*.env"]
    );
    assert_eq!(setup.command, "");
}

#[test]
fn a_worktree_setup_without_files_or_command_is_none() {
    let mut model = ReadModel::default();
    run(&mut model, create_project("p", "/repo")).unwrap();
    run(&mut model, set_setup(&["x"], "bun install")).unwrap();

    run(&mut model, set_setup(&[" ", ""], " ")).unwrap();

    assert_eq!(setup_of(&model), None);
}

#[test]
fn a_worktree_setup_refuses_files_outside_the_project() {
    let mut model = ReadModel::default();
    run(&mut model, create_project("p", "/repo")).unwrap();

    for path in [
        "/etc/passwd",
        r"\\server\share",
        r"C:\Users\nxtvoid\.env",
        "d:secrets",
        "../other/.env",
        "config/../../.env",
        ".git/config",
        ".GIT",
    ] {
        let error = run(&mut model, set_setup(&[".env", path], "")).unwrap_err();
        assert_eq!(
            error,
            CommandError::SetupFileInvalid {
                path: path.to_owned()
            },
            "{path}"
        );
    }
    assert_eq!(setup_of(&model), None);
}

#[test]
fn a_project_keeps_the_settings_it_sets_for_itself() {
    let mut model = ReadModel::default();
    run(&mut model, create_project("p", "/repo")).unwrap();
    assert_eq!(
        model.project("p").unwrap().settings,
        ProjectSettings::default()
    );
    let settings = ProjectSettings {
        continue_after_restart: Some(false),
    };

    run(
        &mut model,
        Command::ProjectSettingsSet {
            project_id: "p".into(),
            settings: settings.clone(),
        },
    )
    .unwrap();

    assert_eq!(model.project("p").unwrap().settings, settings);
    let missing = run(
        &mut model,
        Command::ProjectSettingsSet {
            project_id: "q".into(),
            settings,
        },
    );
    assert_eq!(
        missing,
        Err(CommandError::ProjectNotFound {
            project_id: "q".into()
        })
    );
}

#[test]
fn removing_queued_messages_marks_them_never_read() {
    let model = model_with_thread();
    assert_eq!(
        drop_queued_messages(&model, "t", vec!["m2".into()]),
        [Event::MessagesDropped {
            thread_id: "t".into(),
            message_ids: vec!["m2".into()],
        }]
    );
    assert!(drop_queued_messages(&model, "t", Vec::new()).is_empty());
    assert!(drop_queued_messages(&model, "gone", vec!["m2".into()]).is_empty());
}

fn with_session(model: &mut ReadModel) {
    run(model, send("m1", "work")).unwrap();
    record(
        model,
        "m1",
        ProviderOutput::SessionStarted {
            session_id: "s1".into(),
        },
    );
}

#[test]
fn only_a_resumable_thread_nobody_waits_on_goes_on_alone() {
    let mut model = model_with_thread();
    assert!(!may_continue_alone(&model, "t"));
    with_session(&mut model);
    assert!(may_continue_alone(&model, "t"));

    let question = Question {
        header: "Color".into(),
        question: "Which color?".into(),
        options: Vec::new(),
        multi_select: false,
    };
    let questions = ItemContent::Questions {
        questions: vec![question],
        status: RequestStatus::Pending,
        answers: None,
    };
    open(&mut model, "r1", questions);
    assert!(may_continue_alone(&model, "t"));
    open(&mut model, "r2", approval());
    assert!(!may_continue_alone(&model, "t"));

    let mut model = model_with_thread();
    with_session(&mut model);
    run(&mut model, archive(true)).unwrap();
    assert!(!may_continue_alone(&model, "t"));
    assert!(!may_continue_alone(&model, "gone"));
}

#[test]
fn a_session_a_failed_resume_replaced_comes_back() {
    let mut model = model_with_thread();
    with_session(&mut model);
    assert!(restore_session(&model, "t", "s1").is_empty());

    record(
        &mut model,
        "m1",
        ProviderOutput::SessionStarted {
            session_id: "fresh".into(),
        },
    );
    let events = restore_session(&model, "t", "s1");
    apply_all(&mut model, &events);

    assert_eq!(model.thread("t").unwrap().session_id.as_deref(), Some("s1"));
}

fn retrying(attempt: u32) -> ProviderOutput {
    ProviderOutput::Retrying {
        attempt,
        max_attempts: Some(10),
        retry_in_ms: Some(30_000),
        error: Some("overloaded".into()),
    }
}

#[test]
fn a_retry_holds_until_the_model_answers() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "go")).unwrap();

    record(&mut model, "m1", retrying(1));
    record(&mut model, "m1", retrying(3));
    record(
        &mut model,
        "m1",
        ProviderOutput::WorkingDirectory {
            path: "/repo".into(),
        },
    );

    assert_eq!(
        model.retry("t"),
        Some(&TurnRetry {
            attempt: 3,
            max_attempts: Some(10),
            retry_in_ms: Some(30_000),
            error: Some("overloaded".into()),
            at: NOW.into(),
        })
    );
    let events = record(
        &mut model,
        "m1",
        ProviderOutput::TextDelta {
            item_id: "a".into(),
            delta: "Hi".into(),
        },
    );
    assert_eq!(
        events,
        [
            Event::TurnRetrying {
                thread_id: "t".into(),
                retry: None,
            },
            Event::ItemAdded {
                item: ThreadItem {
                    id: format!("m1:recovered:{NOW}"),
                    thread_id: "t".into(),
                    created_at: NOW.into(),
                    content: ItemContent::ProviderRecovered {
                        attempt: 3,
                        max_attempts: Some(10),
                    },
                },
            },
        ]
    );
    assert_eq!(model.retry("t"), None);
    let later = record(
        &mut model,
        "m1",
        ProviderOutput::AssistantMessage {
            item_id: "a".into(),
            text: "Hi".into(),
        },
    );
    assert!(
        !later
            .iter()
            .any(|event| matches!(event, Event::TurnRetrying { .. }))
    );
}

#[test]
fn a_retry_ends_with_its_turn() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "go")).unwrap();
    record(&mut model, "m1", retrying(2));
    record(&mut model, "x", retrying(1));

    record(
        &mut model,
        "m1",
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Failed {
                reason: TurnFailure::Overloaded {
                    detail: "529".into(),
                },
            },
        },
    );

    assert_eq!(model.retry("t"), None);
    assert!(record(&mut model, "m1", retrying(3)).is_empty());
}

#[test]
fn dropped_messages_are_marked_by_id() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "go")).unwrap();

    let events = record(
        &mut model,
        "m1",
        ProviderOutput::MessagesDropped {
            message_ids: vec!["m2".into()],
        },
    );

    assert_eq!(
        events,
        [Event::MessagesDropped {
            thread_id: "t".into(),
            message_ids: vec!["m2".into()],
        }]
    );
    let none = record(
        &mut model,
        "m1",
        ProviderOutput::MessagesDropped {
            message_ids: Vec::new(),
        },
    );
    assert!(none.is_empty());
}

#[test]
fn a_rerouted_model_and_a_failed_mcp_server_leave_a_notice_in_the_turn() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "go")).unwrap();

    let rerouted = record(
        &mut model,
        "m1",
        ProviderOutput::ModelRerouted {
            model: "gpt-5.1-codex-mini".into(),
            reason: Some("high demand".into()),
        },
    );
    let failed = record(
        &mut model,
        "m1",
        ProviderOutput::McpServerFailed {
            name: "hoshi".into(),
            error: "connection closed".into(),
        },
    );

    assert!(matches!(
        rerouted.as_slice(),
        [Event::ItemAdded { item }]
            if item.id == "m1:rerouted"
                && item.content == ItemContent::ModelRerouted {
                    model: "gpt-5.1-codex-mini".into(),
                    reason: Some("high demand".into()),
                }
    ));
    assert!(matches!(
        failed.as_slice(),
        [Event::ItemAdded { item }]
            if item.id == "m1:mcp:hoshi"
                && item.content == ItemContent::McpServerFailed {
                    name: "hoshi".into(),
                    error: "connection closed".into(),
                }
    ));
}

#[test]
fn a_retry_the_model_never_answered_leaves_no_note() {
    let mut model = model_with_thread();
    run(&mut model, send("m1", "go")).unwrap();
    record(&mut model, "m1", retrying(2));

    let events = record(&mut model, "m1", ProviderOutput::ProcessEnded);

    assert_eq!(
        events,
        [Event::TurnRetrying {
            thread_id: "t".into(),
            retry: None,
        }]
    );
}

#[test]
fn a_failed_mcp_server_shows_once_per_thread() {
    let mut model = model_with_thread();
    let failed = |name: &str| ProviderOutput::McpServerFailed {
        name: name.into(),
        error: "connection closed".into(),
    };
    run(&mut model, send("m1", "go")).unwrap();
    assert_eq!(record(&mut model, "m1", failed("github")).len(), 1);
    record(
        &mut model,
        "m1",
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        },
    );

    run(&mut model, send("m2", "again")).unwrap();

    assert!(record(&mut model, "m2", failed("github")).is_empty());
    assert!(matches!(
        record(&mut model, "m2", failed("linear")).as_slice(),
        [Event::ItemAdded { item }] if item.id == "m2:mcp:linear"
    ));
}

#[test]
fn a_resumed_turn_opens_with_hoshis_message_marked_as_such() {
    let mut model = model_with_thread();

    let events = start_resumed_turn(
        &model,
        "t",
        "r1",
        "Continue where you left off.",
        Resumption::Restart,
        NOW,
    );
    apply_all(&mut model, &events);

    assert!(matches!(
        events.as_slice(),
        [
            Event::ItemAdded { item },
            Event::TurnStarted { turn_id, .. },
        ] if item.id == "r1"
            && turn_id == "r1"
            && matches!(
                &item.content,
                ItemContent::UserMessage {
                    resumed: Some(Resumption::Restart),
                    rewind: None,
                    steer: false,
                    ..
                }
            )
    ));
    let busy = start_resumed_turn(&model, "t", "r2", "x", Resumption::Restart, NOW);
    assert!(busy.is_empty());
}
