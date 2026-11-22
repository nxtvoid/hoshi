use super::*;
use crate::{AccessMode, ProviderSession, ThreadStatus};

fn item(id: &str, content: ItemContent) -> ThreadItem {
    ThreadItem {
        id: id.into(),
        thread_id: "t".into(),
        created_at: "2026-11-15T10:00:00Z".into(),
        content,
    }
}

fn user(id: &str, text: &str, rewind: Option<Rewind>) -> ThreadItem {
    item(
        id,
        ItemContent::UserMessage {
            text: text.into(),
            attachments: Vec::new(),
            steer: false,
            task_result: None,
            pull_request_update: None,
            rewind,
            dropped: false,
            resumed: None,
            model_switch: None,
        },
    )
}

fn answer(id: &str, text: &str) -> ThreadItem {
    item(id, ItemContent::AssistantMessage { text: text.into() })
}

fn command(id: &str, line: &str, output: &str, failed: bool) -> ThreadItem {
    item(
        id,
        ItemContent::ToolCall {
            name: "Bash".into(),
            input: format!(r#"{{"command":"{line}"}}"#),
            output: Some(output.into()),
            is_error: failed,
            detail: None,
            output_omitted: false,
            interrupted: false,
        },
    )
}

fn after(cursor: &str, session: &str) -> Option<Rewind> {
    Some(Rewind::After {
        cursor: cursor.into(),
        session: Some(session.into()),
    })
}

pub(crate) fn thread() -> Thread {
    Thread {
        id: "t".into(),
        project_id: Some("p".into()),
        title: "Thread".into(),
        status: ThreadStatus::Idle,
        branch: None,
        worktree_path: None,
        session_id: Some("s".into()),
        conversation_cursor: Some("c2".into()),
        resume_at: None,
        watching_pull_request: false,
        provider: "claudeCode".into(),
        model: None,
        effort: None,
        fast_mode: false,
        ultracode: false,
        access_mode: AccessMode::default(),
        plan_mode: false,
        active_turn_id: None,
        archived: false,
        pinned: false,
        worktree_released: false,
        worktree_adopted: false,
        parent_thread_id: None,
        task_cancelled: false,
        task_answered: false,
        forked_from: None,
        pending_model: None,
        provider_sessions: Vec::new(),
        pending_handoffs: Vec::new(),
        snoozed_until: None,
        snoozed_at: None,
        last_snooze_wake_at: None,
        last_visited_at: None,
        last_turn_ended_at: None,
        created_at: "2026-11-15T10:00:00Z".into(),
        updated_at: "2026-11-15T10:00:00Z".into(),
        last_turn_model: None,
    }
}

fn two_turns() -> Vec<ThreadItem> {
    vec![
        user("m1", "Remember APPLE.", Some(Rewind::Start)),
        command("b1", "ls", "a.txt", false),
        answer("a1", "OK"),
        user("m2", "Remember BANANA.", after("c1", "s")),
        answer("a2", "OK"),
    ]
}

#[test]
fn a_fork_point_ends_its_turn_with_the_cursor_the_next_turn_started_from() {
    let items = two_turns();
    let thread = thread();

    let first = turn_point(
        &thread,
        &items,
        &ForkPoint::Turn {
            item_id: "a1".into(),
        },
    )
    .unwrap();
    let latest = turn_point(&thread, &items, &ForkPoint::LatestStable).unwrap();

    assert_eq!(first.turn_id, "m1");
    assert_eq!(first.through_item_id, "a1");
    assert_eq!(first.ordinal, 1);
    assert_eq!(first.cursor.as_deref(), Some("c1"));
    assert_eq!(first.session_id.as_deref(), Some("s"));
    assert_eq!(latest.turn_id, "m2");
    assert_eq!(latest.cursor.as_deref(), Some("c2"));
}

#[test]
fn a_running_turn_is_no_stable_point_and_its_turn_says_so() {
    let mut thread = thread();
    thread.active_turn_id = Some("m2".into());
    let items = two_turns();

    let latest = turn_point(&thread, &items, &ForkPoint::LatestStable).unwrap();
    let running = turn_point(
        &thread,
        &items,
        &ForkPoint::Turn {
            item_id: "a2".into(),
        },
    )
    .unwrap();

    assert_eq!(latest.turn_id, "m1");
    assert_eq!(running.status, TurnStatus::Running);
}

#[test]
fn a_turn_before_a_provider_switch_belongs_to_the_parked_session() {
    let mut thread = thread();
    thread.provider = "codex".into();
    thread.session_id = Some("x".into());
    thread.provider_sessions = vec![ProviderSession {
        provider: "claudeCode".into(),
        session_id: "s".into(),
        conversation_cursor: Some("c1".into()),
        resume_at: None,
        seen_before: "m2".into(),
    }];
    let mut items = two_turns();
    items[3] = user("m2", "Remember BANANA.", None);

    let first = turn_point(
        &thread,
        &items,
        &ForkPoint::Turn {
            item_id: "a1".into(),
        },
    )
    .unwrap();

    assert_eq!(first.provider, "claudeCode");
    assert_eq!(first.session_id.as_deref(), Some("s"));
    assert_eq!(first.cursor.as_deref(), Some("c1"));
}

#[test]
fn a_failed_turn_and_an_interrupted_one_say_how_they_ended() {
    let items = vec![
        user("m1", "a", Some(Rewind::Start)),
        item(
            "m1:failed",
            ItemContent::TurnFailed {
                reason: TurnFailure::ProviderDisabled,
            },
        ),
        user("m2", "b", None),
        item("m2:interrupted", ItemContent::TurnInterrupted),
        user("m3", "c", None),
    ];

    let statuses: Vec<TurnStatus> = transcript_turns(&items, false)
        .iter()
        .map(|turn| turn.status)
        .collect();

    assert_eq!(
        statuses,
        [
            TurnStatus::Failed,
            TurnStatus::Interrupted,
            TurnStatus::Completed
        ]
    );
    let mut thread = thread();
    thread.conversation_cursor = None;
    let latest = turn_point(&thread, &items[..4], &ForkPoint::LatestStable);
    assert_eq!(
        latest,
        Err(CommandError::NoStableTurn {
            thread_id: "t".into()
        })
    );
    let latest = turn_point(&thread, &items, &ForkPoint::LatestStable).unwrap();
    assert_eq!(latest.turn_id, "m3");
}

#[test]
fn a_handoff_carries_messages_commands_and_file_changes_but_not_reasoning() {
    let mut items = two_turns();
    items.insert(
        2,
        item(
            "r1",
            ItemContent::Reasoning {
                text: "thinking".into(),
            },
        ),
    );
    items.insert(
        3,
        item(
            "e1",
            ItemContent::ToolCall {
                name: "Edit".into(),
                input: r#"{"file_path":"/repo/a.txt"}"#.into(),
                output: Some("ok".into()),
                is_error: false,
                detail: None,
                output_omitted: false,
                interrupted: false,
            },
        ),
    );
    let section = history_section(
        HandoffStrategy::FullThreadSummary,
        "t",
        &items,
        0..items.len(),
    )
    .unwrap();

    let handoff = assemble_handoff(vec![section], Vec::new(), 16_000)
        .unwrap()
        .unwrap();
    let text = handoff.inline_prompt("Go on.");

    assert!(text.starts_with(
        "Context handoff (full_thread_summary):\nProvider context handoff. Thread: t. \
             Covered turns: 1-2.\nSource item range: m1 through a2."
    ));
    assert!(text.contains(
        "\nRecover an omitted item only when the request needs it, with \
             mcp__hoshi__thread_read({threadId:\"t\",itemId:\"<omitted item ID>\",\
             maxCharsPerItem:4000}); continue long text with textOffset=nextTextOffset until \
             null. Items included below need no read. Turn/item IDs"
    ));
    assert!(text.contains("Selected 6 intact items; omitted 0 items."));
    assert!(text.contains(
        "[Historical assistant; command_execution; thread=t; turn=m1; item=b1; \
             status=completed; turn-status=completed]\nCommand: ls\nExit code: 0\na.txt"
    ));
    assert!(text.contains("File change: /repo/a.txt"));
    assert!(!text.contains("thinking"));
    assert!(text.ends_with("\n\nUser message:\nGo on."));
}

#[test]
fn what_does_not_fit_leaves_the_middle_out_and_keeps_the_ends() {
    let mut items = vec![user("first", "the task", Some(Rewind::Start))];
    for index in 0..40 {
        items.push(answer(&format!("a{index}"), &"x".repeat(400)));
    }
    items.push(user("last", "the latest ask", None));
    items.push(answer("final", "the latest answer"));
    let section = history_section(
        HandoffStrategy::FullThreadSummary,
        "t",
        &items,
        0..items.len(),
    )
    .unwrap();

    let handoff = assemble_handoff(vec![section], Vec::new(), 4_000)
        .unwrap()
        .unwrap();
    let kept: Vec<&str> = handoff
        .messages
        .iter()
        .map(|message| message.item_id.as_str())
        .collect();

    assert!(kept.contains(&"first") && kept.contains(&"last") && kept.contains(&"final"));
    assert!(handoff.omitted_items > 0);
    assert!(handoff.context.contains("Omitted item IDs, newest last:"));
    assert!(history_cost(&handoff.messages, &handoff.context) as u64 <= 4_000);
}

#[test]
fn a_handoff_too_wide_to_list_its_sources_still_says_how_to_recover_an_item() {
    let items = two_turns();
    let sections: Vec<HandoffSection> = ["a", "b", "c", "d", "e", "f"]
        .into_iter()
        .filter_map(|thread_id| {
            history_section(
                HandoffStrategy::FullThreadSummary,
                thread_id,
                &items,
                0..items.len(),
            )
        })
        .collect();

    let handoff = assemble_handoff(sections, Vec::new(), 4_000)
        .unwrap()
        .unwrap();

    assert!(handoff.context.starts_with(
        "Context handoff (full_thread_summary). 6 handoff records; detailed coverage \
             references omitted. Recover an omitted item only when the request needs it, with \
             mcp__hoshi__thread_read({threadId:\"f\",itemId:\"<omitted item ID>\",\
             maxCharsPerItem:4000}); continue long text"
    ));
    assert!(!handoff.context.contains("Source item range"));
}

#[test]
fn no_room_at_all_fails_the_handoff() {
    let items = two_turns();
    let section = history_section(
        HandoffStrategy::FullThreadSummary,
        "t",
        &items,
        0..items.len(),
    )
    .unwrap();

    assert_eq!(
        assemble_handoff(vec![section], Vec::new(), 0),
        Err(TurnFailure::ContextHandoffTooLarge)
    );
}

#[test]
fn the_budget_leaves_room_for_the_message_and_a_quarter_of_the_window() {
    let usage = ContextUsage {
        used_tokens: 100_000,
        max_tokens: Some(200_000),
        cache_ttl_secs: None,
        cache_expires_at: None,
    };
    let budget = |cap: u64, usage: Option<ContextUsage>, window: Option<u64>| {
        HandoffBudget {
            token_cap: cap,
            model_window: window,
            usage,
            user_text: "hi",
            attachments: &[],
        }
        .bytes()
    };

    assert_eq!(handoff_token_cap(None), 16_000);
    assert_eq!(handoff_token_cap(Some(10)), 1_024);
    assert_eq!(handoff_token_cap(Some(1_000_000)), 64_000);
    assert_eq!(budget(16_000, None, None), 16_000);
    assert_eq!(
        budget(64_000, Some(usage), None),
        200_000 - 100_000 - 4 - 50_000
    );
    assert_eq!(
        budget(64_000, Some(usage), Some(1_000_000)),
        200_000 - 100_000 - 4 - 50_000
    );
    assert_eq!(budget(64_000, None, Some(40_000)), 40_000 - 4 - 16_000);
    assert_eq!(budget(64_000, None, Some(10_000)), 0);
}

#[test]
fn a_merge_back_lists_the_fork_s_items_in_short_lines() {
    let mut items = two_turns();
    items.push(answer("a3", &"word ".repeat(100)));
    let summary = MergeBackSummary::of("fork", "t", 2, &items);

    let text = summary.text(64_000);

    assert!(text.starts_with(
        "Merge-back context from forked conversation.\nSource thread: fork\n\
             Target thread: t\nCovered fork turns: 1-2\n\nFork delta:\n\
             - User: Remember APPLE.\n- Command: ls\n- Assistant: OK\n"
    ));
    let long = text.lines().last().unwrap();
    assert_eq!(long.chars().count(), "- Assistant: ".len() + 240);
    assert!(long.ends_with("..."));
    let short = summary.text(450);
    assert!(short.contains("earlier items omitted."));
    assert!(short.ends_with(long));
}
