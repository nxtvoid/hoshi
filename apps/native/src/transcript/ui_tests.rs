use super::TranscriptView;
use crate::state::ThreadState;
use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{
    AppContext as _, Bounds, Entity, Modifiers, Point, TestAppContext, VisualTestContext,
    WindowBounds, WindowOptions, px, size,
};
use hoshi_core::{
    CompactionStatus, ItemContent, Rewind, Task, TaskStatus, ThreadItem, TurnFailure, TurnRetry,
};

fn item(id: &str, content: ItemContent) -> ThreadItem {
    ThreadItem {
        id: id.into(),
        thread_id: "t".into(),
        created_at: "2026-11-13T10:00:00Z".into(),
        content,
    }
}

fn user(id: &str, text: &str, task_result: Option<&str>) -> ThreadItem {
    item(
        id,
        ItemContent::UserMessage {
            text: text.into(),
            attachments: Vec::new(),
            steer: false,
            task_result: task_result.map(Into::into),
            pull_request_update: None,
            resumed: None,
            rewind: Some(Rewind::Start),
            dropped: false,
            model_switch: None,
        },
    )
}

fn answer(id: &str) -> ThreadItem {
    item(id, ItemContent::AssistantMessage { text: id.into() })
}

fn call(id: &str, name: &str, input: &str, output: Option<&str>) -> ThreadItem {
    item(
        id,
        ItemContent::ToolCall {
            name: name.into(),
            input: input.into(),
            output: output.map(Into::into),
            is_error: false,
            detail: None,
            output_omitted: false,
            interrupted: false,
        },
    )
}

fn open(
    cx: &mut TestAppContext,
    items: Vec<ThreadItem>,
    working: bool,
) -> (VisualTestContext, Entity<TranscriptView>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        hoshi_ui::theme::apply(ThemeMode::Dark, cx);
    });
    let (window, view) = cx.update(|cx| {
        let thread = cx.new(|_| ThreadState::new("t", items));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(800.), px(600.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| {
            cx.new(|cx| TranscriptView::new(thread, working, cx))
        })
        .expect("test window")
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    redraw(&mut cx);
    (cx, view)
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
}

fn shows(cx: &mut VisualTestContext, selector: &'static str) -> bool {
    cx.debug_bounds(selector).is_some()
}

#[gpui_kit::test]
fn the_task_list_and_compaction_show_and_the_messages_hoshi_sent_do_not(cx: &mut TestAppContext) {
    let items = vec![
        user("u1", "Write the files", None),
        item(
            "l1",
            ItemContent::TaskList {
                tasks: vec![Task {
                    subject: "Write a.txt".into(),
                    active_form: Some("Writing a.txt".into()),
                    status: TaskStatus::InProgress,
                }],
            },
        ),
        user("c1", "/compact", None),
        item(
            "k1",
            ItemContent::Compaction {
                status: CompactionStatus::Done,
                before_tokens: Some(150_000),
                after_tokens: Some(12_000),
                error: None,
            },
        ),
        user("u2", "Go on", None),
        user("r1", "The task answered", Some("child")),
        answer("a1"),
    ];
    let (mut cx, _) = open(cx, items, true);
    assert!(shows(&mut cx, "task-list-l1"));
    assert!(shows(&mut cx, "compaction-k1"));
    assert!(shows(&mut cx, "rewind-u1"));
    assert!(!shows(&mut cx, "rewind-c1"));
    assert!(!shows(&mut cx, "rewind-r1"));
}

#[gpui_kit::test]
fn agents_started_together_are_one_card_that_opens_on_each(cx: &mut TestAppContext) {
    let items = vec![
        user("u1", "Look around", None),
        call(
            "s1",
            "Agent",
            r#"{"description":"Explore","subagent_type":"Explore"}"#,
            None,
        ),
        call(
            "d1",
            "mcp__hoshi__delegate_task",
            r#"{"task":"Fix the build"}"#,
            None,
        ),
    ];
    let (mut cx, _) = open(cx, items, true);
    assert!(shows(&mut cx, "agent-group-agents:s1"));
    assert!(!shows(&mut cx, "subagent-s1"));
    assert!(!shows(&mut cx, "delegation-d1"));

    let header = cx
        .debug_bounds("agent-group-agents:s1")
        .expect("the group shows");
    cx.simulate_click(header.center(), Modifiers::default());
    redraw(&mut cx);
    assert!(shows(&mut cx, "subagent-s1"));
    assert!(shows(&mut cx, "delegation-d1"));
}

#[gpui_kit::test]
fn a_delegated_task_alone_is_its_own_card(cx: &mut TestAppContext) {
    let items = vec![
        user("u1", "Fix it elsewhere", None),
        call(
            "d1",
            "mcp__hoshi__delegate_task",
            r#"{"task":"Fix the build"}"#,
            Some(r#"{"taskId":"child","title":"Fix CI"}"#),
        ),
        answer("a1"),
    ];
    let (mut cx, _) = open(cx, items, false);
    assert!(shows(&mut cx, "delegation-d1"));
}

#[gpui_kit::test]
fn a_reply_to_a_task_answer_folds_as_a_note_and_the_answer_stays_hidden(cx: &mut TestAppContext) {
    let items = vec![
        user("u1", "Fix it elsewhere", None),
        call(
            "d1",
            "mcp__hoshi__delegate_task",
            r#"{"task":"Fix the build"}"#,
            Some(r#"{"taskId":"child","title":"Fix CI"}"#),
        ),
        answer("started"),
        user(
            "r1",
            "Delegated task child (Fix CI) [failed]:\nNo luck",
            Some("child"),
        ),
        answer("noted"),
        answer("summary"),
    ];
    let (mut cx, _) = open(cx, items, false);
    assert!(shows(&mut cx, "fold-row-tasks:u1"));
    assert!(!shows(&mut cx, "task-note-noted"));
    let fold = cx
        .debug_bounds("fold-row-tasks:u1")
        .expect("the settled work folds");
    cx.simulate_click(fold.center(), Modifiers::default());
    redraw(&mut cx);
    assert!(shows(&mut cx, "task-note-noted"));
    assert!(shows(&mut cx, "delegation-d1"));
    assert!(!shows(&mut cx, "rewind-r1"));
}

#[gpui_kit::test]
fn a_pull_request_update_is_one_line_that_opens_to_what_the_agent_was_told(
    cx: &mut TestAppContext,
) {
    let update = item(
        "p1",
        ItemContent::UserMessage {
            text: "Checks failed on #7".into(),
            attachments: Vec::new(),
            steer: false,
            task_result: None,
            pull_request_update: Some(hoshi_core::PullRequestUpdate {
                number: 7,
                failed_checks: vec!["build".into()],
                checks_passed: false,
                comments: 0,
                conflicting: false,
                closed: false,
                stopped: false,
            }),
            resumed: None,
            rewind: Some(Rewind::Start),
            dropped: false,
            model_switch: None,
        },
    );
    let (mut cx, view) = open(cx, vec![update, answer("a1")], false);
    assert!(shows(&mut cx, "pr-update-p1"));
    assert!(!shows(&mut cx, "rewind-p1"));
    let line = cx.debug_bounds("pr-update-p1").expect("the update shows");
    cx.simulate_click(line.center(), Modifiers::default());
    redraw(&mut cx);
    assert!(cx.update(|_, cx| view.read(cx).open.contains("p1")));
}

#[gpui_kit::test]
fn a_page_the_agent_showed_is_a_card_kept_out_of_the_fold(cx: &mut TestAppContext) {
    let items = vec![
        user("u1", "Chart it", None),
        call("c1", "Read", r#"{"file_path":"a.csv"}"#, Some("1,2")),
        call(
            "h1",
            "mcp__hoshi__html_render",
            r#"{"title":"Chart"}"#,
            Some(r#"{"htmlRender":{"id":"ab_cd","title":"Chart","height":300}}"#),
        ),
        answer("a1"),
    ];
    let (mut cx, _) = open(cx, items, false);
    assert!(shows(&mut cx, "fold-row-fold:u1"));
    assert!(shows(&mut cx, "html-page-h1"));
}

#[gpui_kit::test]
fn a_reply_to_a_task_answer_reads_as_a_note_while_it_is_still_written(cx: &mut TestAppContext) {
    let items = vec![
        user("u1", "Fix it elsewhere", None),
        call(
            "d1",
            "mcp__hoshi__delegate_task",
            r#"{"task":"Fix the build"}"#,
            Some(r#"{"taskId":"child","title":"Fix CI"}"#),
        ),
        call(
            "d2",
            "mcp__hoshi__delegate_task",
            r#"{"task":"Write the docs"}"#,
            Some(r#"{"taskId":"other","title":"Docs"}"#),
        ),
        user(
            "r1",
            "Delegated task child (Fix CI) [completed]:\nFixed",
            Some("child"),
        ),
    ];
    let (mut cx, view) = open(cx, items, true);
    view.update(&mut cx, |view, cx| {
        view.tasks = vec![("other".into(), crate::details::agents::AgentState::Running)];
        view.thread.update(cx, |thread, cx| {
            thread.streaming.push(crate::state::Streaming {
                item_id: "s1".into(),
                kind: hoshi_core::StreamKind::Text,
                text: "CI is green again".into(),
            });
            cx.notify();
        });
    });
    redraw(&mut cx);
    assert!(shows(&mut cx, "task-note-s1"));
}

#[gpui_kit::test]
fn a_page_the_agent_showed_is_live_in_the_transcript_and_a_card_when_it_cannot_be(
    cx: &mut TestAppContext,
) {
    use crate::browser::Browsers;
    use crate::panel::RightPanel;
    use crate::state::{self, Workspace};
    use hoshi_core::{AccessMode, Command, ThreadWorkspace};
    use std::sync::Arc;

    cx.executor().allow_parking();
    let home = std::env::temp_dir().join(format!("hoshi-native-page-{}", uuid::Uuid::new_v4()));
    let pages = home.join("attachments").join("pages").join("ab");
    std::fs::create_dir_all(&pages).unwrap();
    std::fs::write(pages.join("ab_cd.html"), "<p>hi</p>").unwrap();
    let (notify, _engine_notifications) = state::channel();
    let (_, notifications) = state::channel();
    let engine = Arc::new(hoshi_engine::Engine::open(&home, notify).unwrap());
    engine
        .dispatch(Command::ThreadCreate {
            thread_id: "t".into(),
            project_id: None,
            title: "chat".into(),
            workspace: ThreadWorkspace::Local,
            provider: hoshi_core::default_provider(),
            model: None,
            effort: None,
            fast_mode: false,
            ultracode: false,
            access_mode: AccessMode::default(),
            plan_mode: false,
        })
        .unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        hoshi_ui::theme::apply(ThemeMode::Dark, cx);
        crate::markdown::init(engine.clone(), home.clone(), cx);
    });
    let workspace = cx.update(|cx| cx.new(|cx| Workspace::new(engine, notifications, cx)));
    let (browsers, sent) = Browsers::detached();
    let browsers = cx.update(|cx| cx.new(|_| browsers));
    workspace.update(cx, |workspace, _| workspace.browsers = browsers.clone());
    let items = vec![
        user("u1", "Show it", None),
        call(
            "h1",
            "mcp__hoshi__html_render",
            r#"{"title":"Tasks"}"#,
            Some(r#"{"htmlRender":{"id":"ab_cd","title":"Tasks","height":300}}"#),
        ),
        answer("a1"),
    ];
    let (window, _) = cx
        .update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(800.), px(600.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                let panel = cx.new(|cx| RightPanel::new(workspace.clone(), window, cx));
                crate::markdown::attach(&workspace, &panel, cx);
                let thread = cx.new(|_| ThreadState::new("t", items));
                cx.new(|cx| TranscriptView::new(thread, false, cx))
            })
        })
        .unwrap();
    let mut cx = VisualTestContext::from_window(window, cx);
    redraw(&mut cx);

    assert!(shows(&mut cx, "html-page-view"));
    let row = cx.debug_bounds("html-page-h1").unwrap();
    assert_eq!(row.size.height, px(300.));
    let asked = sent.take();
    let key = "t#html-page:ab_cd";
    assert!(
        asked
            .iter()
            .any(|request| request == &format!("{key}: View(true)")),
        "{asked:?}"
    );
    assert!(
        asked
            .iter()
            .any(|request| request.starts_with(&format!("{key}: Fit("))),
        "{asked:?}"
    );
    assert!(
        asked
            .iter()
            .any(|request| request.starts_with(&format!("{key}: Scale("))),
        "{asked:?}"
    );
    let opened: Vec<_> = asked
        .iter()
        .filter(|request| request.starts_with(&format!("{key}: Open(")))
        .collect();
    assert_eq!(opened.len(), 1, "{asked:?}");
    assert!(opened[0].contains("ab_cd.html#hoshi-theme=%7B%22appearance%22%3A%22dark%22"));

    redraw(&mut cx);
    assert!(
        sent.take().iter().all(|request| !request.contains("Open(")),
        "a page shown again is not opened again"
    );

    browsers.update(&mut cx, |browsers, cx| browsers.fail(key, cx));
    redraw(&mut cx);
    assert!(!shows(&mut cx, "html-page-view"));
    assert!(shows(&mut cx, "html-page-h1"));
}

#[gpui_kit::test]
fn a_page_whose_output_was_cut_shows_once_read_whole(cx: &mut TestAppContext) {
    let full = r#"{"htmlRender":{"id":"ab_cd","title":"Tasks","height":300}}"#;
    let mut cut = call("h1", "mcp__hoshi__html_render", "{}", Some(&full[..20]));
    if let ItemContent::ToolCall { output_omitted, .. } = &mut cut.content {
        *output_omitted = true;
    }
    let items = vec![user("u1", "Show it", None), cut, answer("a1")];
    let (mut cx, view) = open(cx, items, false);
    let thread = view.read_with(&cx, |view, _| view.thread.clone());
    assert_eq!(
        thread.read_with(&cx, |thread, _| crate::html_page::pages_to_read(thread)),
        vec!["h1".to_owned()]
    );
    assert!(!shows(&mut cx, "html-page-h1"));

    thread.update(&mut cx, |thread, cx| {
        let whole = call("h1", "mcp__hoshi__html_render", "{}", Some(full)).content;
        thread.whole.insert("h1".into(), whole);
        cx.notify();
    });
    redraw(&mut cx);
    assert!(shows(&mut cx, "html-page-h1"));
    assert!(
        thread
            .read_with(&cx, |thread, _| crate::html_page::pages_to_read(thread))
            .is_empty()
    );
}

fn dropped_steer(id: &str, text: &str) -> ThreadItem {
    item(
        id,
        ItemContent::UserMessage {
            text: text.into(),
            attachments: Vec::new(),
            steer: true,
            task_result: None,
            pull_request_update: None,
            resumed: None,
            rewind: None,
            dropped: true,
            model_switch: None,
        },
    )
}

#[gpui_kit::test]
fn a_message_the_turn_never_read_says_so_and_goes_back_to_the_composer(cx: &mut TestAppContext) {
    let items = vec![
        user("u1", "Write the files", None),
        answer("a1"),
        dropped_steer("s1", "Use tabs"),
    ];
    let (mut cx, view) = open(cx, items, false);
    assert!(shows(&mut cx, "not-sent-s1"));
    assert!(!shows(&mut cx, "not-sent-u1"));

    let restored = std::rc::Rc::new(std::cell::RefCell::new(None));
    let _subscription = cx.update({
        let restored = restored.clone();
        |_, cx| {
            cx.subscribe(&view, move |_, event: &crate::rewind::Rewound, _| {
                *restored.borrow_mut() = Some(event.0.blocks[0].text.clone());
            })
        }
    });
    let button = cx.debug_bounds("send-again-s1").expect("the button shows");
    cx.simulate_click(button.center(), Modifiers::default());
    redraw(&mut cx);
    assert_eq!(restored.borrow().as_deref(), Some("Use tabs"));
}

#[gpui_kit::test]
fn a_failed_sign_in_offers_to_sign_in_again_while_it_is_the_latest(cx: &mut TestAppContext) {
    let failed = item(
        "f1",
        ItemContent::TurnFailed {
            reason: TurnFailure::AuthenticationFailed {
                detail: "401".into(),
            },
        },
    );
    let (mut cx, view) = open(cx, vec![user("u1", "Go", None), failed.clone()], false);
    assert!(shows(&mut cx, "turn-failed-f1"));

    let asked = std::rc::Rc::new(std::cell::RefCell::new(None));
    let _subscription = cx.update({
        let asked = asked.clone();
        |_, cx| {
            cx.subscribe(
                &view,
                move |_, event: &crate::request_rows::OpenProviderSettings, _| {
                    *asked.borrow_mut() = Some(event.0);
                },
            )
        }
    });
    let button = cx.debug_bounds("sign-in-f1").expect("the button shows");
    cx.simulate_click(button.center(), Modifiers::default());
    redraw(&mut cx);
    assert_eq!(*asked.borrow(), Some(hoshi_core::DEFAULT_PROVIDER));

    let thread = view.read_with(&cx, |view, _| view.thread.clone());
    thread.update(&mut cx, |thread, cx| {
        thread.items.push(user("u2", "Again", None));
        cx.notify();
    });
    redraw(&mut cx);
    assert!(shows(&mut cx, "turn-failed-f1"));
    assert!(!shows(&mut cx, "sign-in-f1"));
}

#[gpui_kit::test]
fn a_turn_on_another_effort_reads_as_a_divider_over_its_message(cx: &mut TestAppContext) {
    use hoshi_core::{Effort, ModelChoice, ModelSwitch};
    let choice = |effort| ModelChoice {
        provider: "claudeCode".into(),
        model: Some("claude-opus-5-5".into()),
        effort: Some(effort),
        fast_mode: false,
        ultracode: false,
    };
    let switched = |id: &str, from, to| {
        let mut message = user(id, id, None);
        if let ItemContent::UserMessage { model_switch, .. } = &mut message.content {
            *model_switch = Some(Box::new(ModelSwitch {
                from: choice(from),
                to: choice(to),
            }));
        }
        message
    };
    let (mut cx, _) = open(
        cx,
        vec![
            switched("u1", Effort::Medium, Effort::High),
            answer("a1"),
            switched("u2", Effort::High, Effort::High),
            answer("a2"),
            item(
                "q1:model",
                ItemContent::ModelSwitched {
                    from: choice(Effort::High),
                    to: choice(Effort::Medium),
                },
            ),
        ],
        false,
    );
    assert!(shows(&mut cx, "model-switch-u1"));
    assert!(!shows(&mut cx, "model-switch-u2"));
    assert!(shows(&mut cx, "model-switch-q1:model"));
    let text = crate::request_rows::model_switch_text(
        &ModelSwitch {
            from: choice(Effort::Medium),
            to: choice(Effort::High),
        },
        None,
    );
    assert_eq!(text.as_deref(), Some("Medium → High"));
}

#[gpui_kit::test]
fn a_provider_handoff_reads_as_a_divider_in_the_turn(cx: &mut TestAppContext) {
    use hoshi_core::{HandoffDelivery, HandoffSource, HandoffStrategy, ModelChoice};
    let choice = |provider: &str| ModelChoice {
        provider: provider.into(),
        model: None,
        effort: None,
        fast_mode: false,
        ultracode: false,
    };
    let handoff = item(
        "turn:handoff",
        ItemContent::ContextHandoff {
            sources: vec![HandoffSource::ProviderSwitch {
                from: choice("claudeCode"),
                to: choice("codex"),
                strategy: HandoffStrategy::FullThreadSummary,
            }],
            delivery: HandoffDelivery::Failed,
            included_items: 3,
            omitted_items: 0,
        },
    );
    let (mut cx, _) = open(cx, vec![user("u1", "Go on", None), handoff], false);
    assert!(shows(&mut cx, "handoff-turn:handoff"));
}

#[gpui_kit::test]
fn a_usage_limit_offers_to_resume_and_snooze_until_its_reset_while_it_is_the_latest(
    cx: &mut TestAppContext,
) {
    use crate::panel::RightPanel;
    use crate::state::{self, Workspace};
    use std::sync::Arc;

    cx.executor().allow_parking();
    let home = std::env::temp_dir().join(format!("hoshi-native-limit-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    let (notify, notifications) = state::channel();
    let engine = Arc::new(hoshi_engine::Engine::open(&home, notify).unwrap());
    cx.update(|cx| {
        gpui_kit::init(cx);
        hoshi_ui::theme::apply(ThemeMode::Dark, cx);
        crate::markdown::init(engine.clone(), home.clone(), cx);
    });
    let workspace = cx.update(|cx| cx.new(|cx| Workspace::new(engine, notifications, cx)));
    let limited = |id: &str, resets_at: chrono::DateTime<chrono::Utc>| {
        item(
            id,
            ItemContent::TurnFailed {
                reason: TurnFailure::UsageLimited {
                    detail: "limit".into(),
                    resets_at: Some(resets_at.to_rfc3339()),
                },
            },
        )
    };
    let now = chrono::Utc::now();
    let items = vec![
        user("u1", "Go", None),
        limited("f1", now - chrono::Duration::hours(1)),
        user("u2", "Again", None),
        limited("f2", now + chrono::Duration::hours(1)),
    ];
    let (window, view) = cx
        .update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(800.), px(600.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                let panel = cx.new(|cx| RightPanel::new(workspace.clone(), window, cx));
                crate::markdown::attach(&workspace, &panel, cx);
                let thread = cx.new(|_| ThreadState::new("t", items));
                cx.new(|cx| TranscriptView::new(thread, false, cx))
            })
        })
        .unwrap();
    let mut cx = VisualTestContext::from_window(window, cx);
    redraw(&mut cx);
    assert!(shows(&mut cx, "limit-resume-f2"));
    assert!(shows(&mut cx, "limit-snooze-f2"));
    assert!(!shows(&mut cx, "limit-resume-f1"));

    let thread = view.read_with(&cx, |view, _| view.thread.clone());
    thread.update(&mut cx, |thread, cx| {
        thread.items.push(user("u3", "Later", None));
        cx.notify();
    });
    redraw(&mut cx);
    assert!(shows(&mut cx, "turn-failed-f2"));
    assert!(!shows(&mut cx, "limit-resume-f2"));
    assert!(!shows(&mut cx, "limit-snooze-f2"));
}

struct CachedShell {
    transcript: Entity<TranscriptView>,
    room: hoshi_ui::LaneRoom,
}

impl gpui_kit::Render for CachedShell {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        cx: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::{ParentElement as _, Styled as _};
        let room_changed = hoshi_ui::LaneRoom::set(self.room, cx);
        gpui_kit::div()
            .size_full()
            .flex()
            .flex_col()
            .child(super::element(self.transcript.clone(), room_changed))
    }
}

#[gpui_kit::test]
fn the_cached_transcript_follows_the_lanes_room_and_a_limits_reset_passing(
    cx: &mut TestAppContext,
) {
    use crate::panel::RightPanel;
    use crate::state::{self, Workspace};
    use std::sync::Arc;
    use std::time::Duration;

    cx.executor().allow_parking();
    let home = std::env::temp_dir().join(format!("hoshi-native-cached-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    let (notify, notifications) = state::channel();
    let engine = Arc::new(hoshi_engine::Engine::open(&home, notify).unwrap());
    cx.update(|cx| {
        gpui_kit::init(cx);
        hoshi_ui::theme::apply(ThemeMode::Dark, cx);
        crate::markdown::init(engine.clone(), home.clone(), cx);
    });
    let workspace = cx.update(|cx| cx.new(|cx| Workspace::new(engine, notifications, cx)));
    let resets_at = chrono::Utc::now() + chrono::Duration::seconds(3);
    let items = vec![
        user("u1", "Go", None),
        item(
            "f1",
            ItemContent::TurnFailed {
                reason: TurnFailure::UsageLimited {
                    detail: "limit".into(),
                    resets_at: Some(resets_at.to_rfc3339()),
                },
            },
        ),
    ];
    let (window, shell) = cx
        .update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(800.), px(600.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                let panel = cx.new(|cx| RightPanel::new(workspace.clone(), window, cx));
                crate::markdown::attach(&workspace, &panel, cx);
                let thread = cx.new(|_| ThreadState::new("t", items));
                let transcript = cx.new(|cx| TranscriptView::new(thread, false, cx));
                cx.new(|_| CachedShell {
                    transcript,
                    room: hoshi_ui::LaneRoom::default(),
                })
            })
        })
        .unwrap();
    let mut cx = VisualTestContext::from_window(window, cx);
    redraw(&mut cx);
    assert!(shows(&mut cx, "limit-resume-f1"));

    let left = |cx: &mut VisualTestContext| cx.debug_bounds("turn-failed-f1").unwrap().left();
    let before = left(&mut cx);
    shell.update(&mut cx, |shell, cx| {
        shell.room.gutter = px(200.);
        cx.notify();
    });
    redraw(&mut cx);
    assert!(left(&mut cx) > before + px(100.), "the lane moves right");

    let wait = (resets_at - chrono::Utc::now())
        .to_std()
        .unwrap_or_default();
    std::thread::sleep(wait + Duration::from_millis(200));
    cx.executor().advance_clock(Duration::from_secs(30));
    redraw(&mut cx);
    assert!(shows(&mut cx, "turn-failed-f1"));
    assert!(!shows(&mut cx, "limit-resume-f1"));
}

#[gpui_kit::test]
fn a_rerouted_model_and_a_failed_mcp_server_show_in_the_turn(cx: &mut TestAppContext) {
    let items = vec![
        user("u1", "Go", None),
        item(
            "u1:rerouted",
            ItemContent::ModelRerouted {
                model: "gpt-5.1-codex-mini".into(),
                reason: Some("high demand".into()),
            },
        ),
        item(
            "u1:mcp:hoshi",
            ItemContent::McpServerFailed {
                name: "hoshi".into(),
                error: "connection closed".into(),
            },
        ),
    ];
    let (mut cx, _) = open(cx, items, true);
    assert!(shows(&mut cx, "notice-u1:rerouted"));
    assert!(shows(&mut cx, "notice-u1:mcp:hoshi"));
    assert!(
        shows(&mut cx, "notice-u1:mcp:hoshi-detail"),
        "the error shows apart, as written"
    );
}

#[gpui_kit::test]
fn a_turn_hoshi_resumed_opens_with_a_notice_and_a_recovered_provider_leaves_one(
    cx: &mut TestAppContext,
) {
    let resumed = item(
        "r1",
        ItemContent::UserMessage {
            text: "Continue where you left off.".into(),
            attachments: Vec::new(),
            steer: false,
            task_result: None,
            pull_request_update: None,
            resumed: Some(hoshi_core::Resumption::Restart),
            rewind: None,
            dropped: false,
            model_switch: None,
        },
    );
    let items = vec![
        user("u1", "Go", None),
        answer("a1"),
        resumed,
        item(
            "r1:recovered",
            ItemContent::ProviderRecovered {
                attempt: 3,
                max_attempts: Some(10),
            },
        ),
        answer("a2"),
    ];
    let (mut cx, _) = open(cx, items, false);
    assert!(shows(&mut cx, "notice-r1"));
    assert!(shows(&mut cx, "notice-r1:recovered"));
    assert!(!shows(&mut cx, "rewind-r1"));
}

#[gpui_kit::test]
fn a_retry_shows_beside_the_status_until_it_clears(cx: &mut TestAppContext) {
    let (mut cx, view) = open(cx, vec![user("u1", "Go", None)], true);
    assert!(!shows(&mut cx, "turn-retry"));

    view.update(&mut cx, |view, cx| {
        view.set_retry(
            Some(TurnRetry {
                attempt: 2,
                max_attempts: Some(10),
                retry_in_ms: Some(30_000),
                error: Some("overloaded".into()),
                at: "2026-11-13T10:00:00Z".into(),
            }),
            cx,
        )
    });
    redraw(&mut cx);
    assert!(shows(&mut cx, "turn-retry"));

    view.update(&mut cx, |view, cx| view.set_retry(None, cx));
    redraw(&mut cx);
    assert!(!shows(&mut cx, "turn-retry"));
}
