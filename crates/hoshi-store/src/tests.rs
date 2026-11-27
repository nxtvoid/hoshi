use super::*;
use hoshi_core::{
    AccessMode, BankedResets, ChecksState, Command, CompactionStatus, ForgeKind, ItemContent,
    Mergeability, ProviderOutput, PullRequest, PullRequestState, ReadModel, ToolDetail,
    UsageWindow, UsageWindowKind, decide, record_output,
};

const NOW: &str = "2026-11-07T12:00:00.000Z";

fn project_created(id: &str) -> Event {
    Event::ProjectCreated {
        project: Project {
            id: id.into(),
            title: id.into(),
            workspace_root: format!("/repo/{id}"),
            created_at: NOW.into(),
            updated_at: NOW.into(),
            worktree_setup: None,
            settings: hoshi_core::ProjectSettings::default(),
        },
    }
}

#[test]
fn a_worktree_setup_saved_before_files_existed_still_loads() {
    let mut store = Store::open_in_memory().unwrap();
    store.append(&[project_created("p")], NOW).unwrap();
    store
        .conn
        .execute(
            "UPDATE projects SET worktree_setup = ?1 WHERE id = 'p'",
            [r#"{"command":"bun install","wait":true}"#],
        )
        .unwrap();

    assert_eq!(
        store.snapshot().unwrap().projects[0].worktree_setup,
        Some(hoshi_core::WorktreeSetup {
            files: Vec::new(),
            command: "bun install".into(),
            wait: true,
        })
    );
}

#[test]
fn a_project_keeps_the_settings_it_sets_for_itself() {
    let mut store = Store::open_in_memory().unwrap();
    store.append(&[project_created("p")], NOW).unwrap();
    store
        .conn
        .execute("UPDATE projects SET settings = NULL WHERE id = 'p'", [])
        .unwrap();
    assert_eq!(
        store.snapshot().unwrap().projects[0].settings,
        hoshi_core::ProjectSettings::default()
    );
    let settings = hoshi_core::ProjectSettings {
        continue_after_restart: Some(false),
    };

    store
        .append(
            &[Event::ProjectSettingsSet {
                project_id: "p".into(),
                settings: settings.clone(),
            }],
            NOW,
        )
        .unwrap();

    assert_eq!(store.snapshot().unwrap().projects[0].settings, settings);
}

#[test]
fn a_project_keeps_its_worktree_setup() {
    let mut store = Store::open_in_memory().unwrap();
    let setup = hoshi_core::WorktreeSetup {
        files: vec![".env".into(), "secrets/*.env".into()],
        command: "bun install".into(),
        wait: true,
    };
    store
        .append(
            &[
                project_created("p"),
                Event::ProjectWorktreeSetupSet {
                    project_id: "p".into(),
                    setup: Some(setup.clone()),
                },
            ],
            NOW,
        )
        .unwrap();
    assert_eq!(
        store.snapshot().unwrap().projects[0].worktree_setup,
        Some(setup)
    );

    store
        .append(
            &[Event::ProjectWorktreeSetupSet {
                project_id: "p".into(),
                setup: None,
            }],
            NOW,
        )
        .unwrap();
    assert_eq!(store.snapshot().unwrap().projects[0].worktree_setup, None);
}

#[test]
fn a_thread_s_recovery_is_kept_until_forgotten_or_its_thread_goes() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .append(
            &[
                project_created("p"),
                thread_created("t", "p"),
                thread_created("u", "p"),
            ],
            NOW,
        )
        .unwrap();
    let queued = serde_json::json!({ "queued": [{ "id": "m2", "text": "also this" }] });
    store.save_thread_recovery("t", Some(&queued)).unwrap();
    store.save_thread_recovery("u", Some(&queued)).unwrap();
    let read: Vec<(String, serde_json::Value)> = store.thread_recoveries().unwrap();
    assert_eq!(read.len(), 2);
    assert!(read.contains(&("t".to_owned(), queued.clone())));

    store
        .save_thread_recovery::<serde_json::Value>("u", None)
        .unwrap();
    store
        .append(
            &[Event::ThreadDeleted {
                thread_id: "t".into(),
            }],
            NOW,
        )
        .unwrap();
    let read: Vec<(String, serde_json::Value)> = store.thread_recoveries().unwrap();
    assert!(read.is_empty());
}

#[test]
fn a_pull_request_watch_is_kept_until_its_thread_goes() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .append(&[project_created("p"), thread_created("t", "p")], NOW)
        .unwrap();
    let watch = PullRequestWatch {
        number: 7,
        head_sha: Some("abc".into()),
        failed_checks: vec!["lint".into()],
        remarks_through: NOW.into(),
        ..PullRequestWatch::default()
    };
    store.save_pull_request_watch("t", &watch).unwrap();
    assert_eq!(
        store.pull_request_watches().unwrap(),
        [("t".to_owned(), watch)]
    );

    store
        .append(
            &[Event::ThreadDeleted {
                thread_id: "t".into(),
            }],
            NOW,
        )
        .unwrap();
    assert!(store.pull_request_watches().unwrap().is_empty());
}

#[test]
fn upgrading_forgets_the_events_of_threads_deleted_before() {
    let mut conn = Connection::open_in_memory().unwrap();
    let before = MIGRATIONS
        .iter()
        .position(|migration| migration.contains("DELETE FROM events"))
        .unwrap();
    let tx = conn.transaction().unwrap();
    for migration in &MIGRATIONS[..before] {
        tx.execute_batch(migration).unwrap();
    }
    tx.execute(
        "INSERT INTO threads (id, project_id, title, status, created_at, updated_at)
             VALUES ('t', 'p', 'kept', 'idle', ?1, ?1)",
        [NOW],
    )
    .unwrap();
    let events = [
        (
            "projectCreated",
            r#"{"type":"projectCreated","project":{"id":"p"}}"#,
        ),
        (
            "threadTitleSet",
            r#"{"type":"threadTitleSet","threadId":"t","title":"kept"}"#,
        ),
        (
            "itemAdded",
            r#"{"type":"itemAdded","item":{"threadId":"t:sub"}}"#,
        ),
        (
            "threadTitleSet",
            r#"{"type":"threadTitleSet","threadId":"gone","title":"secret"}"#,
        ),
        (
            "itemAdded",
            r#"{"type":"itemAdded","item":{"threadId":"gone:sub"}}"#,
        ),
        (
            "threadDeleted",
            r#"{"type":"threadDeleted","threadId":"gone"}"#,
        ),
    ];
    for (event_type, payload) in events {
        tx.execute(
            "INSERT INTO events (event_type, payload_json, occurred_at) VALUES (?1, ?2, ?3)",
            params![event_type, payload, NOW],
        )
        .unwrap();
    }
    tx.pragma_update(None, "user_version", before as i64)
        .unwrap();
    tx.commit().unwrap();

    let store = Store::init(conn).unwrap();

    let kept: Vec<String> = store
        .conn
        .prepare("SELECT coalesce(thread_id, '-') FROM events ORDER BY sequence")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(kept, ["-", "t", "t:sub", "gone"]);
}

fn thread_created(id: &str, project_id: &str) -> Event {
    Event::ThreadCreated {
        thread: Box::new(Thread {
            id: id.into(),
            project_id: Some(project_id.into()),
            title: String::new(),
            status: ThreadStatus::WaitingForInput,
            branch: Some("feat/x".into()),
            worktree_path: Some("/worktrees/x".into()),
            session_id: None,
            conversation_cursor: None,
            resume_at: None,
            watching_pull_request: false,
            provider: "copilot".into(),
            model: Some("sonnet".into()),
            effort: Some(Effort::High),
            fast_mode: true,
            ultracode: false,
            access_mode: AccessMode::Auto,
            plan_mode: true,
            active_turn_id: None,
            archived: false,
            pinned: true,
            worktree_released: false,
            worktree_adopted: false,
            parent_thread_id: None,
            task_cancelled: false,
            task_answered: false,
            snoozed_until: Some("2026-11-08T09:00:00.000Z".into()),
            snoozed_at: Some(NOW.into()),
            last_snooze_wake_at: Some("2026-11-06T09:00:00.000Z".into()),
            last_visited_at: Some("2026-11-06T10:00:00.000Z".into()),
            last_turn_ended_at: Some("2026-11-06T11:00:00.000Z".into()),
            forked_from: None,
            pending_model: None,
            provider_sessions: Vec::new(),
            pending_handoffs: Vec::new(),
            created_at: NOW.into(),
            updated_at: NOW.into(),
            last_turn_model: None,
        }),
    }
}

#[test]
fn snapshot_previews_the_latest_message_of_delegated_tasks() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .append(
            &[
                project_created("p"),
                thread_created("t", "p"),
                thread_created("task", "p"),
                Event::ThreadDelegated {
                    thread_id: "task".into(),
                    parent_thread_id: "t".into(),
                    shares_worktree: false,
                },
                answer("t", "parent says"),
                answer("task", "first"),
                item(
                    "task",
                    "m2",
                    ItemContent::AssistantMessage {
                        text: "Counted
  42 files"
                            .into(),
                    },
                ),
            ],
            NOW,
        )
        .unwrap();

    let snapshot = store.snapshot().unwrap();

    assert_eq!(
        snapshot.task_previews,
        [TaskPreview {
            thread_id: "task".into(),
            text: "Counted 42 files".into(),
        }]
    );
}

#[test]
fn snapshot_reflects_appended_events() {
    let mut store = Store::open_in_memory().unwrap();

    let sequences = store
        .append(&[project_created("p"), thread_created("t", "p")], NOW)
        .unwrap();

    assert_eq!(sequences, [1, 2]);
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.sequence, 2);
    assert_eq!(snapshot.projects.len(), 1);
    let Event::ThreadCreated { thread } = thread_created("t", "p") else {
        unreachable!()
    };
    assert_eq!(snapshot.threads, [*thread]);
}

#[test]
fn a_chat_is_kept_without_a_project_and_outlives_project_removal() {
    let mut store = Store::open_in_memory().unwrap();
    let Event::ThreadCreated { mut thread } = thread_created("chat", "p") else {
        unreachable!()
    };
    thread.project_id = None;
    store
        .append(
            &[
                project_created("p"),
                Event::ThreadCreated {
                    thread: thread.clone(),
                },
                Event::ProjectRemoved {
                    project_id: "p".into(),
                },
            ],
            NOW,
        )
        .unwrap();

    assert_eq!(store.snapshot().unwrap().threads, [*thread]);
}

#[test]
fn the_last_merged_cleanup_is_replaced_by_the_next() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.merged_cleanup::<serde_json::Value>().unwrap(), None);
    store
        .save_merged_cleanup(&serde_json::json!({ "removed": 1 }))
        .unwrap();
    store
        .save_merged_cleanup(&serde_json::json!({ "removed": 2 }))
        .unwrap();
    assert_eq!(
        store.merged_cleanup().unwrap(),
        Some(serde_json::json!({ "removed": 2 }))
    );
}

#[test]
fn threads_from_before_chats_keep_their_project() {
    let mut conn = Connection::open_in_memory().unwrap();
    let tx = conn.transaction().unwrap();
    for migration in &MIGRATIONS[..MIGRATIONS.len() - 1] {
        tx.execute_batch(migration).unwrap();
    }
    tx.execute(
        "INSERT INTO threads (id, project_id, title, status, created_at, updated_at, pinned)
             VALUES ('t', 'p', 'old', 'idle', ?1, ?1, 1)",
        [NOW],
    )
    .unwrap();
    tx.pragma_update(None, "user_version", MIGRATIONS.len() as i64 - 1)
        .unwrap();
    tx.commit().unwrap();

    let store = Store::init(conn).unwrap();

    let threads = store.snapshot().unwrap().threads;
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].project_id.as_deref(), Some("p"));
    assert!(threads[0].pinned);
    assert_eq!(threads[0].snoozed_until, None);
    assert_eq!(threads[0].last_turn_ended_at, None);
}

fn commit(store: &mut Store, model: &mut ReadModel, events: Vec<Event>) {
    events.iter().for_each(|event| model.apply(event));
    store.append(&events, NOW).unwrap();
}

fn store_with_thread() -> (Store, ReadModel) {
    let mut store = Store::open_in_memory().unwrap();
    store
        .append(&[project_created("p"), thread_created("t", "p")], NOW)
        .unwrap();
    let model = ReadModel::from_snapshot(&store.snapshot().unwrap());
    (store, model)
}

#[test]
fn a_cached_detail_survives_a_restart_and_goes_with_its_thread() {
    let (mut store, model) = store_with_thread();
    let cached = CachedPullRequestDetail {
        thread_id: "t".into(),
        number: 3,
        version: NOW.into(),
        detail: PullRequestDetail {
            supports: hoshi_core::PullRequestSupport::all(),
            forge_id: "PR_1".into(),
            repository: "o/r".into(),
            mergeability: Mergeability::Mergeable,
            merge_methods: Vec::new(),
            can_edit: true,
            body: "## Summary".into(),
            changed_files: 2,
            reviewers: Vec::new(),
            labels: Vec::new(),
            checks: Vec::new(),
            review_threads: Vec::new(),
            timeline: Vec::new(),
        },
    };

    store.save_pull_request_detail(&cached).unwrap();
    assert_eq!(store.pull_request_details().unwrap(), [cached]);

    let delete = Command::ThreadDelete {
        thread_id: "t".into(),
    };
    store
        .append(&decide(&model, delete, NOW).unwrap(), NOW)
        .unwrap();
    assert!(store.pull_request_details().unwrap().is_empty());
}

#[test]
fn a_cached_pull_request_survives_a_restart_and_goes_with_its_thread() {
    let (mut store, model) = store_with_thread();
    let update = ThreadPullRequest {
        thread_id: "t".into(),
        pull_request: Some(PullRequest {
            forge: ForgeKind::Github,
            number: 42,
            title: "Fix".into(),
            url: "https://github.com/o/r/pull/42".into(),
            author: Some("me".into()),
            head_branch: "feat/x".into(),
            head_sha: None,
            base_branch: "main".into(),
            state: PullRequestState::Open,
            checks: Some(ChecksState::Failing),
            unresolved_comments: 3,
            open_tasks: 0,
            changes_requested: false,
            turn: None,
            conflicts: false,
            additions: 1,
            deletions: 2,
            updated_at: NOW.into(),
        }),
    };

    store.save_pull_request(&update).unwrap();
    assert_eq!(store.snapshot().unwrap().pull_requests, [update]);

    let delete = Command::ThreadDelete {
        thread_id: "t".into(),
    };
    store
        .append(&decide(&model, delete, NOW).unwrap(), NOW)
        .unwrap();
    assert!(store.snapshot().unwrap().pull_requests.is_empty());
}

#[test]
fn a_context_usage_survives_a_restart_and_goes_with_its_thread() {
    let (mut store, model) = store_with_thread();
    let first = ThreadContextUsage {
        thread_id: "t".into(),
        usage: ContextUsage {
            used_tokens: 9_000,
            max_tokens: None,
            ..ContextUsage::default()
        },
    };
    let latest = ThreadContextUsage {
        thread_id: "t".into(),
        usage: ContextUsage {
            used_tokens: 12_000,
            max_tokens: Some(1_000_000),
            ..ContextUsage::default()
        },
    };

    store.save_context_usage(&first).unwrap();
    store.save_context_usage(&latest).unwrap();
    assert_eq!(store.snapshot().unwrap().context_usages, [latest]);

    let delete = Command::ThreadDelete {
        thread_id: "t".into(),
    };
    store
        .append(&decide(&model, delete, NOW).unwrap(), NOW)
        .unwrap();
    assert!(store.snapshot().unwrap().context_usages.is_empty());
}

fn usage_limits(used_percent: u8) -> UsageLimits {
    UsageLimits {
        windows: vec![UsageWindow {
            kind: UsageWindowKind::Session,
            model: None,
            used_percent,
            window_minutes: 300,
            resets_at: Some("2026-11-08T12:00:00Z".into()),
        }],
        banked_resets: Some(BankedResets {
            count: 1,
            next_grant_id: Some("grant_a".into()),
            expires_at: vec!["2026-12-01T00:00:00Z".into()],
            grants: Vec::new(),
        }),
        account: None,
        extra_usage: None,
    }
}

#[test]
fn each_providers_last_usage_limits_replace_only_its_previous_ones() {
    let mut store = Store::open_in_memory().unwrap();
    assert!(store.provider_usage_limits().unwrap().is_empty());

    store
        .save_provider_usage_limits("claudeCode", &usage_limits(20))
        .unwrap();
    store
        .save_provider_usage_limits("codex", &usage_limits(60))
        .unwrap();
    store
        .save_provider_usage_limits("claudeCode", &usage_limits(35))
        .unwrap();

    let limits = store.provider_usage_limits().unwrap();
    assert_eq!(limits.len(), 2);
    assert_eq!(limits["claudeCode"], usage_limits(35));
    assert_eq!(limits["codex"], usage_limits(60));
}

#[test]
fn the_usage_limits_from_before_providers_become_claude_codes() {
    let mut conn = Connection::open_in_memory().unwrap();
    let before = MIGRATIONS
        .iter()
        .position(|migration| migration.contains("CREATE TABLE provider_usage_limits"))
        .unwrap();
    let tx = conn.transaction().unwrap();
    for migration in &MIGRATIONS[..before] {
        tx.execute_batch(migration).unwrap();
    }
    tx.execute(
        "INSERT INTO usage_limits (id, limits_json) VALUES (1, ?1)",
        [serde_json::to_string(&usage_limits(42)).unwrap()],
    )
    .unwrap();
    tx.pragma_update(None, "user_version", before as i64)
        .unwrap();
    tx.commit().unwrap();

    let store = Store::init(conn).unwrap();

    let limits = store.provider_usage_limits().unwrap();
    assert_eq!(limits.len(), 1);
    assert_eq!(limits["claudeCode"], usage_limits(42));
}

#[test]
fn an_adopted_worktree_survives_a_restart_and_leaving_it_clears_it() {
    let (mut store, mut model) = store_with_thread();
    let attached = hoshi_core::record_worktree(
        &model,
        "t",
        "/home/me/repo-pr".into(),
        Some("feat/pr".into()),
        true,
    );
    commit(&mut store, &mut model, attached);

    let thread = &store.snapshot().unwrap().threads[0];
    assert_eq!(thread.worktree_path.as_deref(), Some("/home/me/repo-pr"));
    assert!(thread.worktree_adopted);

    let left = hoshi_core::record_worktree_left(&model, "t", Some("main".into()));
    commit(&mut store, &mut model, left);

    let thread = &store.snapshot().unwrap().threads[0];
    assert_eq!(thread.worktree_path, None);
    assert_eq!(thread.branch.as_deref(), Some("main"));
    assert!(!thread.worktree_adopted);
}

#[test]
fn archiving_survives_a_restart() {
    let (mut store, mut model) = store_with_thread();
    let command = Command::ThreadArchive {
        thread_id: "t".into(),
        archived: true,
    };

    let events = decide(&model, command, NOW).unwrap();
    commit(&mut store, &mut model, events);

    let thread = &store.snapshot().unwrap().threads[0];
    assert!(thread.archived);
    assert!(!thread.pinned);
}

#[test]
fn a_snooze_its_visits_and_its_wake_survive_a_restart() {
    let (mut store, mut model) = store_with_thread();
    let thread_id = || "t".to_owned();
    let snooze = Command::ThreadSnooze {
        thread_id: thread_id(),
        until: "2026-11-07T15:00:00.000Z".into(),
    };
    let steps = [
        (snooze, NOW),
        (
            Command::ThreadVisit {
                thread_id: thread_id(),
            },
            "2026-11-07T16:00:00.000Z",
        ),
        (
            Command::ThreadPin {
                thread_id: thread_id(),
                pinned: true,
            },
            "2026-11-07T17:00:00.000Z",
        ),
    ];
    for (command, at) in steps {
        let events = decide(&model, command, at).unwrap();
        store.append(&events, at).unwrap();
        for event in &events {
            model.apply(event);
        }
        let expected: Vec<_> = model.threads().cloned().collect();
        assert_eq!(store.snapshot().unwrap().threads, expected);
    }

    let thread = &store.snapshot().unwrap().threads[0];
    assert_eq!(thread.snoozed_until, None);
    assert_eq!(thread.snoozed_at, None);
    assert_eq!(
        thread.last_visited_at.as_deref(),
        Some("2026-11-07T16:00:00.000Z")
    );
    assert_eq!(
        thread.last_snooze_wake_at.as_deref(),
        Some("2026-11-07T17:00:00.000Z")
    );
}

#[test]
fn a_turn_left_unseen_stays_unseen_after_a_restart_until_a_visit() {
    let (mut store, mut model) = store_with_thread();
    let ended = "2026-11-07T13:00:00.000Z";
    let turn_end = Event::TurnCompleted {
        thread_id: "t".into(),
        turn_id: "turn-1".into(),
        outcome: hoshi_core::TurnOutcome::Completed,
        at: ended.into(),
    };
    store
        .append(std::slice::from_ref(&turn_end), ended)
        .unwrap();
    model.apply(&turn_end);

    let reloaded = ReadModel::from_snapshot(&store.snapshot().unwrap());
    assert!(reloaded.thread("t").unwrap().has_unseen_turn());

    let visited = "2026-11-07T14:00:00.000Z";
    let visit = Command::ThreadVisit {
        thread_id: "t".into(),
    };
    let events = decide(&model, visit, visited).unwrap();
    store.append(&events, visited).unwrap();

    let reloaded = ReadModel::from_snapshot(&store.snapshot().unwrap());
    let thread = reloaded.thread("t").unwrap();
    assert_eq!(thread.last_visited_at.as_deref(), Some(visited));
    assert!(!thread.has_unseen_turn());
}

#[test]
fn turns_that_ended_before_visits_were_kept_count_as_seen() {
    let mut conn = Connection::open_in_memory().unwrap();
    let tx = conn.transaction().unwrap();
    for migration in &MIGRATIONS[..MIGRATIONS.len() - 1] {
        tx.execute_batch(migration).unwrap();
    }
    for (id, visited, ended) in [
        ("never", None, Some("2026-11-07T13:00:00.000Z")),
        (
            "before",
            Some("2026-11-07T11:00:00.000Z"),
            Some("2026-11-07T13:00:00.000Z"),
        ),
        (
            "after",
            Some("2026-11-07T15:00:00.000Z"),
            Some("2026-11-07T13:00:00.000Z"),
        ),
        ("idle", None, None),
    ] {
        tx.execute(
            "INSERT INTO threads (id, project_id, title, status, created_at, updated_at,
                 last_visited_at, last_turn_ended_at)
                 VALUES (?1, 'p', 'old', 'idle', ?2, ?2, ?3, ?4)",
            rusqlite::params![id, NOW, visited, ended],
        )
        .unwrap();
    }
    tx.pragma_update(None, "user_version", MIGRATIONS.len() as i64 - 1)
        .unwrap();
    tx.commit().unwrap();

    let store = Store::init(conn).unwrap();

    let threads = store.snapshot().unwrap().threads;
    let visited = |id: &str| {
        let thread = threads.iter().find(|thread| thread.id == id).unwrap();
        assert!(!thread.has_unseen_turn(), "{id}");
        thread.last_visited_at.clone()
    };
    assert_eq!(
        visited("never").as_deref(),
        Some("2026-11-07T13:00:00.000Z")
    );
    assert_eq!(
        visited("before").as_deref(),
        Some("2026-11-07T13:00:00.000Z")
    );
    assert_eq!(
        visited("after").as_deref(),
        Some("2026-11-07T15:00:00.000Z")
    );
    assert_eq!(visited("idle"), None);
}

fn send_text(message_id: &str) -> Command {
    Command::MessageSend {
        thread_id: "t".into(),
        message_id: message_id.into(),
        text: "Read the file".into(),
        attachments: Vec::new(),
    }
}

fn send(model: &ReadModel) -> Vec<Event> {
    let command = Command::MessageSend {
        thread_id: "t".into(),
        message_id: "m1".into(),
        text: "Read the file".into(),
        attachments: Vec::new(),
    };
    decide(model, command, NOW).unwrap()
}

#[test]
fn removing_a_project_deletes_its_threads_and_items() {
    let (mut store, mut model) = store_with_thread();
    let events = send(&model);
    commit(&mut store, &mut model, events);

    let remove = Command::ProjectRemove {
        project_id: "p".into(),
    };
    let events = decide(&model, remove, NOW).unwrap();
    commit(&mut store, &mut model, events);

    assert!(store.snapshot().unwrap().threads.is_empty());
    assert!(store.thread_items("t").unwrap().is_empty());
}

#[test]
fn thread_projection_matches_the_read_model_through_a_turn() {
    let (mut store, mut model) = store_with_thread();
    let events = send(&model);
    commit(&mut store, &mut model, events);
    let model_set = Command::ThreadModelSet {
        thread_id: "t".into(),
        provider: "copilot".into(),
        model: Some("opus".into()),
        effort: Some(Effort::Max),
        fast_mode: true,
        ultracode: true,
    };
    let events = decide(&model, model_set, NOW).unwrap();
    commit(&mut store, &mut model, events);
    let access_set = Command::ThreadAccessSet {
        thread_id: "t".into(),
        access_mode: AccessMode::FullAccess,
        plan_mode: false,
    };
    let events = decide(&model, access_set, NOW).unwrap();
    commit(&mut store, &mut model, events);
    let events = hoshi_core::record_branch(&model, "t", Some("main".into()));
    commit(&mut store, &mut model, events);
    let outputs = [
        ProviderOutput::SessionStarted {
            session_id: "s1".into(),
        },
        ProviderOutput::ToolCallStarted {
            item_id: "tool".into(),
            name: "Read".into(),
            input: "{}".into(),
        },
        ProviderOutput::ToolCallCompleted {
            item_id: "tool".into(),
            output: "hello".into(),
            is_error: false,
            detail: Some(change()),
        },
        ProviderOutput::AssistantMessage {
            item_id: "a".into(),
            text: "It says hello".into(),
        },
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        },
    ];

    for output in outputs {
        let events = record_output(&model, "t", "m1", output, NOW);
        commit(&mut store, &mut model, events);
        let expected: Vec<_> = model.threads().cloned().collect();
        assert_eq!(store.snapshot().unwrap().threads, expected);
    }

    let thread = model.thread("t").unwrap();
    assert_eq!(thread.status, ThreadStatus::Idle);
    assert_eq!(thread.session_id.as_deref(), Some("s1"));
    let items = store.thread_items("t").unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(
        items[1].content,
        ItemContent::ToolCall {
            name: "Read".into(),
            input: "{}".into(),
            output: Some("hello".into()),
            is_error: false,
            detail: Some(change()),
            output_omitted: false,
            interrupted: false,
        }
    );
}

#[test]
fn thread_projection_matches_the_read_model_through_a_switch_a_fork_and_a_merge_back() {
    let (mut store, mut model) = store_with_thread();
    let matches = |store: &Store, model: &ReadModel| {
        let expected: Vec<_> = model.threads().cloned().collect();
        assert_eq!(store.snapshot().unwrap().threads, expected);
    };
    let events = send(&model);
    commit(&mut store, &mut model, events);
    for output in [
        ProviderOutput::SessionStarted {
            session_id: "s1".into(),
        },
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        },
    ] {
        let events = record_output(&model, "t", "m1", output, NOW);
        commit(&mut store, &mut model, events);
    }
    let events = hoshi_core::record_conversation_cursor(&model, "t", "c1".into());
    commit(&mut store, &mut model, events);
    let pick = Command::ThreadModelSet {
        thread_id: "t".into(),
        provider: "codex".into(),
        model: Some("gpt-6".into()),
        effort: Some(Effort::High),
        fast_mode: false,
        ultracode: false,
    };
    let events = decide(&model, pick, NOW).unwrap();
    commit(&mut store, &mut model, events);
    matches(&store, &model);
    let second = Command::MessageSend {
        thread_id: "t".into(),
        message_id: "m2".into(),
        text: "Go on".into(),
        attachments: Vec::new(),
    };
    let events = decide(&model, second, NOW).unwrap();
    commit(&mut store, &mut model, events);
    matches(&store, &model);
    assert_eq!(model.thread("t").unwrap().provider, "codex");
    assert_eq!(model.thread("t").unwrap().provider_sessions.len(), 1);

    let turn = hoshi_core::TurnPoint {
        turn_id: "m1".into(),
        through_item_id: "m1".into(),
        ordinal: 1,
        status: hoshi_core::TurnStatus::Completed,
        provider: "claudeCode".into(),
        session_id: Some("s1".into()),
        cursor: Some("c1".into()),
    };
    let fork = Command::ThreadFork {
        thread_id: "t".into(),
        fork_thread_id: "f".into(),
        turn: turn.clone(),
    };
    let events = decide(&model, fork, NOW).unwrap();
    commit(&mut store, &mut model, events);
    matches(&store, &model);
    let events = record_output(
        &model,
        "t",
        "m2",
        ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        },
        NOW,
    );
    commit(&mut store, &mut model, events);
    let merge_back = Command::ThreadMergeBack {
        thread_id: "f".into(),
        turn,
    };
    let events = decide(&model, merge_back, NOW).unwrap();
    commit(&mut store, &mut model, events);
    matches(&store, &model);
    let delivered = model.thread("t").unwrap().pending_handoffs.clone();
    let events = hoshi_core::consume_handoffs(&model, "t", &delivered);
    commit(&mut store, &mut model, events);
    matches(&store, &model);

    let rewind = Command::ThreadRewind {
        thread_id: "t".into(),
        message_id: "m1".into(),
        rewind: hoshi_core::Rewind::Start,
    };
    let events = decide(&model, rewind, NOW).unwrap();
    commit(&mut store, &mut model, events);
    matches(&store, &model);
    assert!(model.thread("t").unwrap().provider_sessions.is_empty());
}

#[test]
fn a_fork_s_inherited_page_ends_at_its_fork_point() {
    let (mut store, mut model) = store_with_thread();
    let events = send(&model);
    commit(&mut store, &mut model, events);
    for (id, text) in [("a1", "one"), ("a2", "two")] {
        let output = ProviderOutput::AssistantMessage {
            item_id: id.into(),
            text: text.into(),
        };
        let events = record_output(&model, "t", "m1", output, NOW);
        commit(&mut store, &mut model, events);
    }

    let (items, _) = store
        .transcript_page_through("t", Some("a1"), None, 20, 600)
        .unwrap();
    let (missing, _) = store
        .transcript_page_through("t", Some("gone"), None, 20, 600)
        .unwrap();

    let ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(ids, ["m1", "a1"]);
    assert!(missing.is_empty());
}

#[test]
fn a_subagent_call_keeps_its_model_and_effort_as_they_are_corrected() {
    let (mut store, mut model) = store_with_thread();
    let events = send(&model);
    commit(&mut store, &mut model, events);
    let subagent = |model: &str, effort| ToolDetail::Subagent {
        model: model.into(),
        effort,
        resume_id: None,
    };
    let outputs = [
        ProviderOutput::ToolCallStarted {
            item_id: "toolu_agent".into(),
            name: "Agent".into(),
            input: "{}".into(),
        },
        ProviderOutput::ToolCallDetail {
            item_id: "toolu_agent".into(),
            detail: subagent("claude-opus-5-5", Some(Effort::Max)),
        },
        ProviderOutput::ToolCallDetail {
            item_id: "toolu_agent".into(),
            detail: subagent("claude-haiku-5-5", Some(Effort::Low)),
        },
    ];
    for output in outputs {
        let events = record_output(&model, "t", "m1", output, NOW);
        commit(&mut store, &mut model, events);
    }
    let detail = |store: &Store| match &store.thread_items("t").unwrap()[1].content {
        ItemContent::ToolCall { detail, .. } => detail.clone(),
        _ => None,
    };
    assert_eq!(
        detail(&store),
        Some(subagent("claude-haiku-5-5", Some(Effort::Low)))
    );

    let done = ProviderOutput::ToolCallCompleted {
        item_id: "toolu_agent".into(),
        output: "found it".into(),
        is_error: false,
        detail: Some(subagent("claude-haiku-5-5", Some(Effort::Low))),
    };
    let events = record_output(&model, "t", "m1", done, NOW);
    commit(&mut store, &mut model, events);
    assert_eq!(
        detail(&store),
        Some(subagent("claude-haiku-5-5", Some(Effort::Low)))
    );
}

#[test]
fn a_rewind_drops_the_message_and_what_followed_and_survives_a_restart() {
    let (mut store, mut model) = store_with_thread();
    let turn = |model: &ReadModel, message_id: &str| {
        let mut events = decide(model, send_text(message_id), NOW).unwrap();
        events.push(Event::ConversationCursorSet {
            thread_id: "t".into(),
            cursor: format!("u-{message_id}"),
        });
        events
    };
    let events = turn(&model, "m1");
    commit(&mut store, &mut model, events);
    let finished = |model: &ReadModel, turn_id: &str| {
        let done = ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        };
        record_output(model, "t", turn_id, done, NOW)
    };
    let events = finished(&model, "m1");
    commit(&mut store, &mut model, events);
    let events = turn(&model, "m2");
    commit(&mut store, &mut model, events);
    let inside = ProviderOutput::Subagent {
        tool_use_id: "toolu_agent".into(),
        output: Box::new(ProviderOutput::AssistantMessage {
            item_id: "a".into(),
            text: "later".into(),
        }),
    };
    let events = record_output(&model, "t", "m2", inside, NOW);
    commit(&mut store, &mut model, events);
    let events = finished(&model, "m2");
    commit(&mut store, &mut model, events);

    let rewind = Command::ThreadRewind {
        thread_id: "t".into(),
        message_id: "m2".into(),
        rewind: hoshi_core::Rewind::After {
            cursor: "u-m1".into(),
            session: None,
        },
    };
    let events = decide(&model, rewind, NOW).unwrap();
    commit(&mut store, &mut model, events);

    let ids: Vec<_> = store
        .thread_items("t")
        .unwrap()
        .into_iter()
        .map(|item| item.id)
        .collect();
    assert_eq!(ids, ["m1"]);
    assert!(store.thread_items("t:toolu_agent").unwrap().is_empty());
    let thread = store.snapshot().unwrap().threads.remove(0);
    assert_eq!(&thread, model.thread("t").unwrap());
    assert_eq!(thread.resume_at.as_deref(), Some("u-m1"));
}

#[test]
fn deleting_a_thread_deletes_its_subagent_transcripts() {
    let (mut store, mut model) = store_with_thread();
    let events = decide(&model, send_text("m1"), NOW).unwrap();
    commit(&mut store, &mut model, events);
    let inside = ProviderOutput::Subagent {
        tool_use_id: "toolu_agent".into(),
        output: Box::new(ProviderOutput::AssistantMessage {
            item_id: "a".into(),
            text: "3 files".into(),
        }),
    };
    let events = record_output(&model, "t", "m1", inside, NOW);
    commit(&mut store, &mut model, events);
    assert_eq!(store.thread_items("t:toolu_agent").unwrap().len(), 1);

    let events = decide(
        &model,
        Command::ThreadDelete {
            thread_id: "t".into(),
        },
        NOW,
    )
    .unwrap();
    commit(&mut store, &mut model, events);

    assert!(store.thread_items("t:toolu_agent").unwrap().is_empty());
}

#[test]
fn a_page_holds_the_latest_turns_and_says_whether_older_ones_remain() {
    let (mut store, mut model) = store_with_thread();
    for turn in 0..5 {
        let events = decide(&model, send_text(&format!("m{turn}")), NOW).unwrap();
        commit(&mut store, &mut model, events);
        let reply = ProviderOutput::AssistantMessage {
            item_id: format!("a{turn}"),
            text: "ok".into(),
        };
        let done = ProviderOutput::TurnCompleted {
            outcome: TurnOutcome::Completed,
        };
        for output in [reply, done] {
            let events = record_output(&model, "t", &format!("m{turn}"), output, NOW);
            commit(&mut store, &mut model, events);
        }
    }
    let ids = |items: &[ThreadItem]| -> Vec<String> {
        items.iter().map(|item| item.id.clone()).collect()
    };

    let (latest, older) = store.transcript_page("t", None, 2, 100).unwrap();
    assert_eq!(ids(&latest), ["m3", "a3", "m4", "a4"]);
    assert!(older);

    let (earlier, older) = store.transcript_page("t", Some("m3"), 2, 100).unwrap();
    assert_eq!(ids(&earlier), ["m1", "a1", "m2", "a2"]);
    assert!(older);

    let (first, older) = store.transcript_page("t", Some("m1"), 2, 100).unwrap();
    assert_eq!(ids(&first), ["m0", "a0"]);
    assert!(!older);

    let (capped, older) = store.transcript_page("t", None, 2, 3).unwrap();
    assert_eq!(ids(&capped), ["a3", "m4", "a4"]);
    assert!(older);
}

fn change() -> ToolDetail {
    ToolDetail::FileChange {
        path: "/repo/a.txt".into(),
        additions: 1,
        deletions: 0,
        diff: Some("@@ -0,0 +1,1 @@\n+a".into()),
    }
}

#[test]
fn reopening_keeps_data_and_does_not_rerun_migrations() {
    let dir = std::env::temp_dir().join(format!("hoshi-store-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("state.sqlite");
    let _ = std::fs::remove_file(&path);

    Store::open(&path)
        .unwrap()
        .append(&[project_created("p")], NOW)
        .unwrap();
    let snapshot = Store::open(&path).unwrap().snapshot().unwrap();

    assert_eq!(snapshot.projects.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

fn query_plan(store: &Store, sql: &str) -> Vec<String> {
    let mut statement = store
        .conn
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap();
    for index in 1..=statement.parameter_count() {
        statement
            .raw_bind_parameter(index, rusqlite::types::Null)
            .unwrap();
    }
    let mut rows = statement.raw_query();
    let mut details = Vec::new();
    while let Some(row) = rows.next().unwrap() {
        details.push(row.get::<_, String>(3).unwrap());
    }
    details
}

#[test]
fn transcript_queries_search_an_index() {
    let store = Store::open_in_memory().unwrap();
    for sql in [
        PENDING_REQUESTS,
        OPEN_TOOL_CALLS,
        THREAD_ITEM,
        ORDINAL_OF,
        TURN_START,
        PAGE_ITEMS,
        HAS_OLDER,
        LAST_ANSWER,
        LAST_PLAN,
        LAST_DECISION,
        ANSWERED_TASKS,
        THREAD_ITEMS,
        LATEST_MESSAGES,
        MESSAGE_COUNT,
        DELETE_THREAD_ITEMS,
        DELETE_THREAD_EVENTS,
        REWIND_ITEMS,
    ] {
        let plan = query_plan(&store, sql);
        let full_scan = plan.iter().any(|step| {
            (step.starts_with("SCAN thread_items") || step.starts_with("SCAN events"))
                && !step.contains("USING")
        });
        assert!(!full_scan, "{sql}\n{plan:#?}");
        assert!(plan.iter().any(|step| step.contains("INDEX")), "{sql}");
    }
    for (sql, index) in [
        (PENDING_REQUESTS, "INDEX thread_items_pending"),
        (OPEN_TOOL_CALLS, "INDEX thread_items_open_tool_calls"),
        (ANSWERED_TASKS, "INDEX thread_items_task_results"),
        (TURN_START, "INDEX thread_items_thread_kind"),
        (LAST_ANSWER, "INDEX thread_items_thread_kind"),
        (LAST_PLAN, "INDEX thread_items_thread_kind"),
        (LAST_DECISION, "INDEX thread_items_thread_kind"),
        (MEDIA_SINCE_COMPACTION, "INDEX thread_items_thread_kind"),
        (PAGE_ITEMS, "INDEX thread_items_thread_ordinal"),
        (THREAD_ITEMS, "INDEX thread_items_thread_ordinal"),
        (LATEST_MESSAGES, "COVERING INDEX thread_items_thread_kind"),
        (MESSAGE_COUNT, "COVERING INDEX thread_items_thread_kind"),
        (DELETE_THREAD_EVENTS, "INDEX events_thread_id"),
    ] {
        let plan = query_plan(&store, sql);
        assert!(
            plan.iter().any(|step| step.contains(index)),
            "{sql}\n{plan:#?}"
        );
    }
    for sql in [
        THREAD_ITEMS,
        LAST_ANSWER,
        LAST_PLAN,
        TURN_START,
        OPEN_TOOL_CALLS,
    ] {
        let plan = query_plan(&store, sql);
        assert!(
            !plan.iter().any(|step| step.contains("TEMP B-TREE")),
            "{sql}"
        );
    }
}

fn item(thread_id: &str, id: &str, content: ItemContent) -> Event {
    Event::ItemAdded {
        item: ThreadItem {
            id: id.into(),
            thread_id: thread_id.into(),
            created_at: NOW.into(),
            content,
        },
    }
}

fn answer(thread_id: &str, id: &str) -> Event {
    item(
        thread_id,
        id,
        ItemContent::AssistantMessage { text: id.into() },
    )
}

fn event_threads(store: &Store) -> Vec<String> {
    store
        .conn
        .prepare("SELECT thread_id FROM events WHERE thread_id IS NOT NULL ORDER BY sequence")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn message_with(thread_id: &str, id: &str, attachments: Vec<MessageAttachment>) -> Event {
    item(
        thread_id,
        id,
        ItemContent::UserMessage {
            text: String::new(),
            attachments,
            steer: false,
            task_result: None,
            pull_request_update: None,
            resumed: None,
            rewind: None,
            dropped: false,
            model_switch: None,
        },
    )
}

fn compaction(thread_id: &str, id: &str, status: CompactionStatus) -> Event {
    item(
        thread_id,
        id,
        ItemContent::Compaction {
            status,
            before_tokens: None,
            after_tokens: None,
            error: None,
        },
    )
}

#[test]
fn what_a_turn_ran_on_is_read_back_in_the_order_turns_ran() {
    let mut store = Store::open_in_memory().unwrap();
    let choice = |effort| hoshi_core::ModelChoice {
        provider: "claudeCode".into(),
        model: Some("opus".into()),
        effort: Some(effort),
        fast_mode: false,
        ultracode: false,
    };
    let started = |turn_id: &str, effort: Option<hoshi_core::Effort>| Event::TurnStarted {
        thread_id: "t".into(),
        turn_id: turn_id.into(),
        at: NOW.into(),
        model: effort.map(choice),
    };
    use hoshi_core::Effort::{High, Low, Medium};
    store
        .append(
            &[
                message_with("t", "m1", Vec::new()),
                started("m1", Some(Medium)),
                answer("t", "a1"),
                message_with("t", "q", Vec::new()),
                message_with("t", "m2", Vec::new()),
                started("m2", Some(High)),
                answer("t", "a2"),
                started("q", Some(Low)),
                answer("t", "aq"),
                message_with("t", "r1", Vec::new()),
                started("r1", None),
                message_with("t", "m3", Vec::new()),
            ],
            NOW,
        )
        .unwrap();
    let before = |store: &Store, item_id: &str| {
        store
            .turn_model_before("t", item_id)
            .unwrap()
            .and_then(|model| model.effort)
    };

    assert_eq!(before(&store, "m1"), None);
    assert_eq!(
        before(&store, "a1"),
        Some(Medium),
        "its own turn, for a fork"
    );
    assert_eq!(
        before(&store, "m2"),
        Some(Medium),
        "a message queued earlier ran after it"
    );
    assert_eq!(before(&store, "aq"), Some(Low));
    assert_eq!(before(&store, "m3"), Some(Low));
    assert_eq!(before(&store, "gone"), None);
    store.append(&[answer("t", "a1")], NOW).unwrap();
    assert_eq!(
        before(&store, "a1"),
        Some(Medium),
        "an item kept again after later turns started still bounds its own turn"
    );

    store
        .append(
            &[Event::ThreadRewound {
                thread_id: "t".into(),
                message_id: "q".into(),
                resume_at: None,
                last_turn_model: Some(choice(Medium)),
            }],
            NOW,
        )
        .unwrap();
    store
        .append(&[message_with("t", "m4", Vec::new())], NOW)
        .unwrap();
    assert_eq!(
        before(&store, "m4"),
        Some(Medium),
        "the turns the rewind took away don't count"
    );

    let in_u = |event: Event| match event {
        Event::TurnStarted {
            turn_id, at, model, ..
        } => Event::TurnStarted {
            thread_id: "u".into(),
            turn_id,
            at,
            model,
        },
        other => other,
    };
    store
        .append(
            &[
                message_with("u", "n1", Vec::new()),
                in_u(started("n1", Some(Medium))),
                message_with("u", "held", Vec::new()),
                message_with("u", "n2", Vec::new()),
                in_u(started("n2", Some(High))),
                in_u(started("held", Some(High))),
                answer("u", "a-held"),
                Event::ThreadRewound {
                    thread_id: "u".into(),
                    message_id: "n2".into(),
                    resume_at: None,
                    last_turn_model: Some(choice(Medium)),
                },
                message_with("u", "n3", Vec::new()),
            ],
            NOW,
        )
        .unwrap();
    assert_eq!(
        store
            .turn_model_before("u", "n3")
            .unwrap()
            .and_then(|model| model.effort),
        Some(Medium),
        "a held message's turn the rewind took away doesn't count, though the message stays"
    );

    let in_v = |event: Event| match event {
        Event::TurnStarted {
            turn_id, at, model, ..
        } => Event::TurnStarted {
            thread_id: "v".into(),
            turn_id,
            at,
            model,
        },
        other => other,
    };
    store
        .append(
            &[
                message_with("v", "o1", Vec::new()),
                in_v(started("o1", Some(Medium))),
                message_with("v", "waits", Vec::new()),
                answer("v", "b1"),
                message_with("v", "o2", Vec::new()),
                in_v(started("o2", Some(High))),
                answer("v", "b2"),
                in_v(started("waits", Some(Low))),
                answer("v", "b-waits"),
                answer("v", "b1"),
            ],
            NOW,
        )
        .unwrap();
    assert_eq!(
        store
            .turn_model_before("v", "b1")
            .unwrap()
            .and_then(|model| model.effort),
        Some(Medium),
        "an item kept again bounds history where it first came, before a message queued \
             ahead of it ran"
    );
}

#[test]
fn a_dropped_message_stays_marked() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .append(
            &[
                message_with("t", "m1", Vec::new()),
                message_with("t", "m2", Vec::new()),
                answer("t", "a"),
            ],
            NOW,
        )
        .unwrap();

    store
        .append(
            &[Event::MessagesDropped {
                thread_id: "t".into(),
                message_ids: vec!["m2".into(), "a".into()],
            }],
            NOW,
        )
        .unwrap();

    let dropped: Vec<_> = store
        .thread_items("t")
        .unwrap()
        .into_iter()
        .map(|item| match item.content {
            ItemContent::UserMessage { dropped, .. } => (item.id, dropped),
            ItemContent::AssistantMessage { .. } => (item.id, false),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(
        dropped,
        [
            ("m1".into(), false),
            ("m2".into(), true),
            ("a".into(), false)
        ]
    );
}

#[test]
fn media_counts_from_the_last_finished_compaction() {
    let mut store = Store::open_in_memory().unwrap();
    let file = |id: &str| MessageAttachment::File {
        id: id.into(),
        name: format!("{id}.pdf"),
        mime_type: "application/pdf".into(),
        size: 1,
    };
    store
        .append(
            &[
                message_with("t", "m1", vec![file("old")]),
                compaction("t", "c1", CompactionStatus::Done),
                message_with(
                    "t",
                    "m2",
                    vec![
                        file("kept"),
                        MessageAttachment::Skill { name: "pdf".into() },
                    ],
                ),
                compaction("t", "c2", CompactionStatus::Failed),
                message_with("t", "m3", vec![file("last")]),
                message_with("other", "m1", vec![file("elsewhere")]),
            ],
            NOW,
        )
        .unwrap();

    assert_eq!(
        store.media_since_compaction("t").unwrap(),
        [file("kept"), file("last")]
    );
}

#[test]
fn deleting_a_thread_deletes_exactly_it_and_its_subagents() {
    let mut store = Store::open_in_memory().unwrap();
    let ids = ["t", "t:a", "t:b:c", "tx", "t0", "t;", "s:t", "s"];
    let items: Vec<_> = ids.iter().map(|id| answer(id, "a")).collect();
    store.append(&items, NOW).unwrap();

    store
        .append(
            &[Event::ThreadDeleted {
                thread_id: "t".into(),
            }],
            NOW,
        )
        .unwrap();

    let remaining: Vec<_> = ids
        .iter()
        .filter(|id| !store.thread_items(id).unwrap().is_empty())
        .copied()
        .collect();
    assert_eq!(remaining, ["tx", "t0", "t;", "s:t", "s"]);
    assert_eq!(event_threads(&store), ["tx", "t0", "t;", "s:t", "s", "t"]);
}

#[test]
fn a_rewind_keeps_other_threads_that_share_the_prefix() {
    let (mut store, mut model) = store_with_thread();
    let events = decide(&model, send_text("m1"), NOW).unwrap();
    commit(&mut store, &mut model, events);
    store
        .append(&[answer("t:agent", "a"), answer("tx", "a")], NOW)
        .unwrap();

    store
        .append(
            &[Event::ThreadRewound {
                thread_id: "t".into(),
                message_id: "m1".into(),
                resume_at: None,
                last_turn_model: None,
            }],
            NOW,
        )
        .unwrap();

    assert!(store.thread_items("t").unwrap().is_empty());
    assert!(store.thread_items("t:agent").unwrap().is_empty());
    assert_eq!(store.thread_items("tx").unwrap().len(), 1);
}

#[test]
fn deleting_a_thread_forgets_its_events_and_the_sequence_keeps_growing() {
    let mut store = Store::open_in_memory().unwrap();
    store
        .append(
            &[
                project_created("p"),
                thread_created("t", "p"),
                thread_created("u", "p"),
                answer("u", "a"),
                answer("t", "a"),
                answer("t:agent", "b"),
            ],
            NOW,
        )
        .unwrap();
    let before = store.snapshot().unwrap().sequence;

    let deleted = store
        .append(
            &[Event::ThreadDeleted {
                thread_id: "t".into(),
            }],
            NOW,
        )
        .unwrap();

    assert_eq!(deleted, [before + 1]);
    assert_eq!(store.snapshot().unwrap().sequence, before + 1);
    assert_eq!(event_threads(&store), ["u", "u", "t"]);
    let next = store.append(&[answer("u", "c")], NOW).unwrap();
    assert_eq!(next, [before + 2]);

    store
        .append(
            &[Event::ThreadDeleted {
                thread_id: "u".into(),
            }],
            NOW,
        )
        .unwrap();
    store
        .conn
        .execute("DELETE FROM events WHERE event_type = 'threadDeleted'", [])
        .unwrap();
    assert_eq!(
        store.append(&[answer("v", "a")], NOW).unwrap(),
        [before + 4]
    );
}

#[test]
fn removing_a_project_forgets_the_events_of_its_threads_only() {
    let mut store = Store::open_in_memory().unwrap();
    let Event::ThreadCreated { thread: mut chat } = thread_created("chat", "p") else {
        unreachable!()
    };
    chat.project_id = None;
    store
        .append(
            &[
                project_created("p"),
                project_created("q"),
                thread_created("t", "p"),
                thread_created("other", "q"),
                Event::ThreadCreated { thread: chat },
                answer("t", "a"),
                answer("t:agent", "b"),
                answer("other", "a"),
                answer("chat", "a"),
            ],
            NOW,
        )
        .unwrap();

    store
        .append(
            &[Event::ProjectRemoved {
                project_id: "p".into(),
            }],
            NOW,
        )
        .unwrap();

    assert_eq!(event_threads(&store), ["other", "chat", "other", "chat"]);
    assert!(store.thread_items("t:agent").unwrap().is_empty());
    assert_eq!(store.thread_items("other").unwrap().len(), 1);
}

#[test]
fn the_latest_messages_skip_tool_calls_and_count_the_earlier_ones() {
    let mut store = Store::open_in_memory().unwrap();
    let tool = |id: &str| {
        item(
            "t",
            id,
            ItemContent::ToolCall {
                name: "Read".into(),
                input: "{}".into(),
                output: None,
                is_error: false,
                detail: None,
                output_omitted: false,
                interrupted: false,
            },
        )
    };
    store
        .append(
            &[
                answer("t", "a1"),
                tool("c1"),
                answer("t", "a2"),
                tool("c2"),
                answer("t:agent", "inside"),
                answer("t", "a3"),
                tool("c3"),
            ],
            NOW,
        )
        .unwrap();
    let ids =
        |items: Vec<ThreadItem>| -> Vec<String> { items.into_iter().map(|item| item.id).collect() };

    let (latest, earlier) = store.latest_messages("t", 2).unwrap();
    assert_eq!(ids(latest), ["a2", "a3"]);
    assert_eq!(earlier, 1);

    let (all, earlier) = store.latest_messages("t", 10).unwrap();
    assert_eq!(ids(all), ["a1", "a2", "a3"]);
    assert_eq!(earlier, 0);
    assert_eq!(store.last_answer("t").unwrap().as_deref(), Some("a3"));
    assert_eq!(store.snapshot().unwrap().open_tool_calls.len(), 3);
}

#[test]
fn a_database_from_before_the_indexes_keeps_its_items_and_events() {
    let mut conn = Connection::open_in_memory().unwrap();
    let before = MIGRATIONS
        .iter()
        .position(|migration| migration.contains("thread_items_thread_ordinal"))
        .unwrap();
    let tx = conn.transaction().unwrap();
    for migration in &MIGRATIONS[..before] {
        tx.execute_batch(migration).unwrap();
    }
    tx.execute(
        "INSERT INTO threads (id, project_id, title, status, created_at, updated_at)
             VALUES ('t', 'p', 'x', 'idle', ?1, ?1)",
        [NOW],
    )
    .unwrap();
    tx.execute(
        "INSERT INTO thread_items (id, thread_id, created_at, content_json)
             VALUES ('a', 't', ?1, '{\"kind\":\"assistantMessage\",\"text\":\"done\"}')",
        [NOW],
    )
    .unwrap();
    tx.execute(
            "INSERT INTO events (event_type, payload_json, occurred_at)
             VALUES ('threadTitleSet', '{\"type\":\"threadTitleSet\",\"threadId\":\"t\",\"title\":\"x\"}', ?1)",
            [NOW],
        )
        .unwrap();
    tx.pragma_update(None, "user_version", before as i64)
        .unwrap();
    tx.commit().unwrap();

    let mut store = Store::init(conn).unwrap();

    assert_eq!(store.last_answer("t").unwrap().as_deref(), Some("done"));
    assert_eq!(event_threads(&store), ["t"]);
    store
        .append(
            &[Event::ThreadDeleted {
                thread_id: "t".into(),
            }],
            NOW,
        )
        .unwrap();
    assert!(store.thread_items("t").unwrap().is_empty());
    assert_eq!(event_threads(&store), ["t"]);
    assert_eq!(store.snapshot().unwrap().sequence, 2);
}

#[test]
fn a_file_database_is_tuned_for_a_desktop_app() {
    let dir = std::env::temp_dir().join(format!("hoshi-store-pragmas-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(&dir.join("state.sqlite")).unwrap();
    let pragma = |name: &str| -> i64 {
        store
            .conn
            .pragma_query_value(None, name, |row| row.get(0))
            .unwrap()
    };

    assert_eq!(pragma("synchronous"), 1);
    assert_eq!(pragma("temp_store"), 2);
    assert_eq!(pragma("journal_size_limit"), JOURNAL_SIZE_LIMIT);
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn upgrades_a_version_one_database() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(MIGRATIONS[0]).unwrap();
    conn.pragma_update(None, "user_version", 1).unwrap();
    conn.execute(
        "INSERT INTO threads VALUES ('t', 'p', 'old', 'idle', NULL, NULL, ?1, ?1)",
        [NOW],
    )
    .unwrap();

    let store = Store::init(conn).unwrap();

    let thread = &store.snapshot().unwrap().threads[0];
    assert_eq!(thread.title, "old");
    assert_eq!(thread.session_id, None);
    assert_eq!(thread.model, None);
}
