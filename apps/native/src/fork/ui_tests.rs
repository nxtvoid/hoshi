use crate::HoshiWindow;
use crate::state::{self, Workspace};
use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{
    AppContext as _, Bounds, Entity, Modifiers, MouseButton, Point, TestAppContext,
    VisualTestContext, WindowBounds, WindowOptions, px, size,
};
use hoshi_core::{
    AccessMode, Command, Event, ForkPoint, ItemContent, PendingHandoff, Rewind, ThreadItem,
    ThreadWorkspace,
};
use hoshi_engine::Engine;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Harness {
    cx: VisualTestContext,
    window: Entity<HoshiWindow>,
    workspace: Entity<Workspace>,
    engine: Arc<Engine>,
    home: PathBuf,
}

fn user(thread_id: &str, id: &str) -> ThreadItem {
    ThreadItem {
        id: id.into(),
        thread_id: thread_id.into(),
        created_at: "2026-11-15T10:00:00Z".into(),
        content: ItemContent::UserMessage {
            text: format!("Question {id}"),
            attachments: Vec::new(),
            steer: false,
            task_result: None,
            pull_request_update: None,
            resumed: None,
            rewind: Some(Rewind::Start),
            dropped: false,
            model_switch: None,
        },
    }
}

fn answer(thread_id: &str, id: &str) -> ThreadItem {
    ThreadItem {
        id: id.into(),
        thread_id: thread_id.into(),
        created_at: "2026-11-15T10:00:01Z".into(),
        content: ItemContent::AssistantMessage {
            text: format!("Answer {id}"),
        },
    }
}

/// Writes items as a finished turn would, through a second connection to the store.
fn record(home: &Path, items: Vec<ThreadItem>) {
    let mut store = hoshi_store::Store::open(&home.join("state.sqlite")).unwrap();
    let events: Vec<Event> = items
        .into_iter()
        .map(|item| Event::ItemAdded { item })
        .collect();
    store.append(&events, "2026-11-15T10:00:02Z").unwrap();
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
    }
}

fn wait_for(h: &mut Harness, what: &str, done: impl Fn(&mut Harness) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        redraw(&mut h.cx);
        if done(h) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn shows(h: &mut Harness, selector: String) -> bool {
    let selector: &'static str = Box::leak(selector.into_boxed_str());
    h.cx.debug_bounds(selector).is_some()
}

fn bounds(h: &mut Harness, selector: String) -> Bounds<gpui_kit::Pixels> {
    let selector: &'static str = Box::leak(selector.into_boxed_str());
    h.cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
}

fn click(h: &mut Harness, selector: String) {
    let at = bounds(h, selector).center();
    h.cx.simulate_click(at, Modifiers::none());
    redraw(&mut h.cx);
}

fn open_id(h: &mut Harness) -> Option<String> {
    h.workspace
        .read_with(&h.cx, |workspace, cx| workspace.open_thread_id(cx))
}

fn open(h: &mut Harness, thread_id: &str) {
    h.workspace.update(&mut h.cx, |workspace, cx| {
        workspace.open_thread(thread_id, cx)
    });
    redraw(&mut h.cx);
}

/// Thread `t`, "Parser quotes", with two answered turns, open in a window.
fn harness(cx: &mut TestAppContext) -> Harness {
    cx.executor().allow_parking();
    let dir = std::env::temp_dir().join(format!("hoshi-fork-{}", uuid::Uuid::new_v4()));
    let home = dir.join("home");
    std::fs::create_dir_all(dir.join("project")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let (notify, notifications) = state::channel();
    let engine = Arc::new(Engine::open(&home, notify).unwrap());
    engine
        .dispatch(Command::ProjectCreate {
            project_id: "p".into(),
            title: "project".into(),
            workspace_root: dir.join("project").to_string_lossy().into_owned(),
        })
        .unwrap();
    engine
        .dispatch(Command::ThreadCreate {
            thread_id: "t".into(),
            project_id: Some("p".into()),
            title: "Parser quotes".into(),
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
    record(
        &home,
        vec![
            user("t", "u1"),
            answer("t", "a1"),
            user("t", "u2"),
            answer("t", "a2"),
        ],
    );
    cx.update(|cx| {
        gpui_kit::init(cx);
        hoshi_ui::DialogLayer::init(cx);
        crate::terminal::init(cx);
        crate::sidebar::init(cx);
        crate::palette::init(cx);
        crate::thread_header::init(cx);
        crate::markdown::init(engine.clone(), home.clone(), cx);
        hoshi_ui::theme::apply(ThemeMode::Light, cx);
    });
    let shared = engine.clone();
    let workspace = cx.update(|cx| cx.new(|cx| Workspace::new(shared, notifications, cx)));
    let (window, shown) = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1200.), px(800.)),
            })),
            ..Default::default()
        };
        let mut shown = None;
        let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
            let view = cx.new(|cx| HoshiWindow::new(workspace.clone(), window, cx));
            shown = Some(view.clone());
            view
        })
        .unwrap();
        (window, shown.unwrap())
    });
    let cx = VisualTestContext::from_window(window, cx);
    let mut h = Harness {
        cx,
        window: shown,
        workspace,
        engine,
        home,
    };
    open(&mut h, "t");
    wait_for(&mut h, "the transcript", |h| shows(h, "fork-a2".into()));
    h
}

fn fork_of(h: &mut Harness, thread_id: &str) -> Option<String> {
    h.workspace.read_with(&h.cx, |workspace, _| {
        workspace
            .model
            .forks_of(thread_id)
            .first()
            .map(|fork| fork.id.clone())
    })
}

#[gpui_kit::test]
fn an_answer_forks_into_a_thread_that_shows_its_history_and_links_back(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    assert!(shows(&mut h, "rewind-u1".into()));
    click(&mut h, "fork-a1".into());
    wait_for(&mut h, "the fork to open", |h| {
        open_id(h).is_some_and(|open| open != "t")
    });
    let fork = open_id(&mut h).unwrap();
    assert_eq!(fork_of(&mut h, "t").as_deref(), Some(fork.as_str()));
    let title = h.workspace.read_with(&h.cx, |workspace, _| {
        workspace.model.thread(&fork).unwrap().title.clone()
    });
    assert_eq!(title, "Parser quotes fork");

    wait_for(&mut h, "the inherited history", |h| {
        shows(h, "forked-from-t".into())
    });
    assert!(shows(&mut h, "collapse-u1".into()));
    assert!(!shows(&mut h, "collapse-u2".into()), "only up to the fork");
    assert!(
        !shows(&mut h, "rewind-u1".into()),
        "inherited history is read-only"
    );
    assert!(!shows(&mut h, "fork-a1".into()));
    let above = bounds(&mut h, "collapse-u1".into());
    let divider = bounds(&mut h, "forked-from-t".into());
    assert!(
        divider.top() > above.bottom(),
        "{divider:?} below {above:?}"
    );

    click(&mut h, "forked-from-link-t".into());
    wait_for(&mut h, "the source to open", |h| {
        open_id(h).as_deref() == Some("t")
    });
    wait_for(&mut h, "the fork's divider", |h| {
        shows(h, format!("forked-into-{fork}"))
    });
    let into = bounds(&mut h, format!("forked-into-{fork}"));
    let answer = bounds(&mut h, "fork-a1".into());
    let next = bounds(&mut h, "collapse-u2".into());
    assert!(
        answer.bottom() <= into.top() && into.bottom() <= next.top(),
        "{answer:?} / {into:?} / {next:?}"
    );

    click(&mut h, format!("forked-into-link-{fork}"));
    wait_for(&mut h, "the fork to open again", |h| {
        open_id(h).as_deref() == Some(fork.as_str())
    });
}

fn merge_back_ready(h: &mut Harness) -> bool {
    let merge_back = h
        .window
        .read_with(&h.cx, |window, _| window.merge_back.clone());
    merge_back.read_with(&h.cx, |merge_back, _| {
        merge_back.shown.as_ref().is_some_and(|shown| shown.ready)
    })
}

#[gpui_kit::test]
fn merge_back_waits_for_a_turn_of_the_fork_then_opens_the_thread_it_came_from(
    cx: &mut TestAppContext,
) {
    let mut h = harness(cx);
    assert!(!shows(&mut h, "merge-back".into()), "not a fork");
    h.engine
        .fork_thread("t", "f", &ForkPoint::LatestStable)
        .unwrap();
    wait_for(&mut h, "the fork", |h| fork_of(h, "t").is_some());
    open(&mut h, "f");
    wait_for(&mut h, "the merge-back button", |h| {
        shows(h, "merge-back".into())
    });
    assert!(!merge_back_ready(&mut h));
    click(&mut h, "merge-back".into());
    assert_eq!(open_id(&mut h).as_deref(), Some("f"));

    record(&h.home, vec![user("f", "u3"), answer("f", "a3")]);
    let merge_back = h
        .window
        .read_with(&h.cx, |window, _| window.merge_back.clone());
    merge_back.update(&mut h.cx, |merge_back, cx| {
        merge_back.shown = None;
        merge_back.follow(cx);
    });
    wait_for(&mut h, "the fork's turn to count", merge_back_ready);
    click(&mut h, "merge-back".into());
    wait_for(&mut h, "the source to open", |h| {
        open_id(h).as_deref() == Some("t")
    });
    let pending = h.workspace.read_with(&h.cx, |workspace, _| {
        workspace
            .model
            .thread("t")
            .unwrap()
            .pending_handoffs
            .clone()
    });
    assert!(
        pending.iter().any(|handoff| matches!(
            handoff,
            PendingHandoff::MergeBack { fork_thread_id, .. } if fork_thread_id == "f"
        )),
        "{pending:?}"
    );

    h.engine
        .fork_thread("t", "g", &ForkPoint::LatestStable)
        .unwrap();
    record(&h.home, vec![user("g", "u4"), answer("g", "a4")]);
    open(&mut h, "g");
    wait_for(&mut h, "the second fork's turn to count", merge_back_ready);
    click(&mut h, "merge-back".into());
    wait_for(&mut h, "the refusal", |h| {
        shows(h, "hoshi-dialog-popup-0".into())
    });
    assert_eq!(open_id(&mut h).as_deref(), Some("g"));
}

#[gpui_kit::test]
fn a_subagent_transcript_offers_no_fork(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    h.cx.update(|_, cx| {
        let mut forkable = super::Forkable::default();
        let subagent = crate::state::ThreadState::new("call", vec![answer("call", "x")]);
        forkable.refresh(&subagent, false, cx);
        assert!(forkable.ids.is_empty());
        let thread = crate::state::ThreadState::new("t", vec![answer("t", "x")]);
        forkable.refresh(&thread, false, cx);
        assert!(forkable.ids.contains("x"));
    });
}

#[gpui_kit::test]
fn the_thread_menu_forks_from_the_latest_finished_turn(cx: &mut TestAppContext) {
    let mut h = harness(cx);
    let row = bounds(&mut h, "thread-t".into()).center();
    h.cx.simulate_mouse_down(row, MouseButton::Right, Modifiers::none());
    h.cx.simulate_mouse_up(row, MouseButton::Right, Modifiers::none());
    redraw(&mut h.cx);
    click(&mut h, "menu-item-thread-fork".into());
    wait_for(&mut h, "the fork to open", |h| {
        open_id(h).is_some_and(|open| open != "t")
    });
    let fork = open_id(&mut h).unwrap();
    let through = h.workspace.read_with(&h.cx, |workspace, _| {
        let thread = workspace.model.thread(&fork).unwrap();
        thread
            .forked_from
            .as_ref()
            .unwrap()
            .turn
            .through_item_id
            .clone()
    });
    assert_eq!(through, "a2");
    wait_for(&mut h, "the whole inherited history", |h| {
        shows(h, "collapse-u2".into()) && shows(h, "forked-from-t".into())
    });
}
