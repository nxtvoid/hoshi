//! What frames of the whole window cost, measured headless. `cargo test -p hoshi-native
//! perf_ -- --ignored --nocapture`, with `CARGO_PROFILE_DEV_OPT_LEVEL=2` for numbers close to
//! a release build.

use crate::HoshiWindow;
use crate::state::{self, Workspace};
use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{
    AppContext as _, Bounds, Entity, Point, TestAppContext, VisualTestContext, WindowBounds,
    WindowOptions, px, size,
};
use hoshi_core::{
    AccessMode, Command, Event, EventEnvelope, ItemContent, Rewind, StreamDelta, StreamKind,
    ThreadItem, ThreadWorkspace,
};
use hoshi_engine::{Engine, Notification, Notify};
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

const NOW: &str = "2026-11-19T10:00:00Z";

struct Harness {
    cx: VisualTestContext,
    workspace: Entity<Workspace>,
    notify: Notify,
}

fn item(id: String, content: ItemContent) -> ThreadItem {
    ThreadItem {
        id,
        thread_id: "t".into(),
        created_at: NOW.into(),
        content,
    }
}

fn answer_text(turn: usize, paragraphs: usize) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "## Turn {turn}: what changed\n");
    for paragraph in 0..paragraphs {
        let _ = writeln!(
            out,
            "Paragraph {paragraph}. The **store** keeps `thread_items` per thread, and \
             `transcript_page` reads the latest turns. See [the docs](https://example.com/docs/{turn}) \
             and `crates/hoshi-store/src/lib.rs` for where the page ends.\n"
        );
        if paragraph % 3 == 1 {
            let _ = writeln!(out, "- `build_rows` groups work\n- `fold_turns` folds it\n");
        }
        if paragraph % 4 == 2 {
            let _ = writeln!(
                out,
                "```rust\nfn main() {{\n    println!(\"rows\");\n}}\n```\n"
            );
        }
    }
    out
}

fn user(id: String, text: String) -> ThreadItem {
    item(
        id,
        ItemContent::UserMessage {
            text,
            attachments: Vec::new(),
            steer: false,
            task_result: None,
            pull_request_update: None,
            resumed: None,
            rewind: Some(Rewind::Start),
            dropped: false,
            model_switch: None,
        },
    )
}

fn call(id: String, output: Option<String>) -> ThreadItem {
    item(
        id,
        ItemContent::ToolCall {
            name: "Read".into(),
            input: r#"{"file_path":"/home/dev/project/src/lib.rs"}"#.into(),
            output,
            is_error: false,
            detail: None,
            output_omitted: false,
            interrupted: false,
        },
    )
}

fn transcript(turns: usize) -> Vec<ThreadItem> {
    let mut items = Vec::new();
    for turn in 0..turns {
        items.push(user(format!("u{turn}"), format!("Look at turn {turn}.")));
        for index in 0..6 {
            items.push(call(format!("c{turn}-{index}"), Some("line\n".repeat(50))));
        }
        items.push(item(
            format!("a{turn}"),
            ItemContent::AssistantMessage {
                text: answer_text(turn, 6),
            },
        ));
    }
    items
}

fn thread(id: &str, title: &str) -> Command {
    Command::ThreadCreate {
        thread_id: id.into(),
        project_id: Some("p".into()),
        title: title.into(),
        workspace: ThreadWorkspace::Local,
        provider: hoshi_core::default_provider(),
        model: None,
        effort: None,
        fast_mode: false,
        ultracode: false,
        access_mode: AccessMode::default(),
        plan_mode: false,
    }
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
    }
}

/// Thread `t` open with a long transcript, among `threads` others in the sidebar.
fn harness(cx: &mut TestAppContext, threads: usize) -> Harness {
    cx.executor().allow_parking();
    let dir = std::env::temp_dir().join(format!("hoshi-perf-{}", uuid::Uuid::new_v4()));
    let home = dir.join("home");
    std::fs::create_dir_all(dir.join("project")).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let (notify, notifications) = state::channel();
    let engine = Arc::new(Engine::open(&home, notify.clone()).unwrap());
    engine
        .dispatch(Command::ProjectCreate {
            project_id: "p".into(),
            title: "project".into(),
            workspace_root: dir.join("project").to_string_lossy().into_owned(),
        })
        .unwrap();
    engine.dispatch(thread("t", "Open thread")).unwrap();
    for index in 0..threads {
        engine
            .dispatch(thread(
                &format!("x{index}"),
                &format!("Thread number {index}"),
            ))
            .unwrap();
    }
    let mut store = hoshi_store::Store::open(&home.join("state.sqlite")).unwrap();
    let events: Vec<Event> = transcript(20)
        .into_iter()
        .map(|item| Event::ItemAdded { item })
        .collect();
    store.append(&events, NOW).unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        hoshi_ui::DialogLayer::init(cx);
        crate::terminal::init(cx);
        crate::sidebar::init(cx);
        crate::palette::init(cx);
        crate::thread_header::init(cx);
        crate::markdown::init(engine.clone(), home.clone(), cx);
        hoshi_ui::theme::apply(ThemeMode::Dark, cx);
    });
    let shared = engine.clone();
    let workspace = cx.update(|cx| cx.new(|cx| Workspace::new(shared, notifications, cx)));
    let window = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1400.), px(900.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| HoshiWindow::new(workspace.clone(), window, cx))
        })
        .unwrap()
        .0
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    workspace.update(&mut cx, |workspace, cx| workspace.open_thread("t", cx));
    for _ in 0..20 {
        redraw(&mut cx);
        std::thread::sleep(Duration::from_millis(20));
    }
    Harness {
        cx,
        workspace,
        notify,
    }
}

fn threads() -> usize {
    std::env::var("PERF_THREADS")
        .ok()
        .and_then(|count| count.parse().ok())
        .unwrap_or(18)
}

fn send(h: &mut Harness, event: Event) {
    (h.notify)(Notification::Event(EventEnvelope { sequence: 0, event }));
    redraw(&mut h.cx);
}

/// Frames the window's animations ask for, as the platform would deliver them.
fn animation_frames(h: &mut Harness, name: &str) {
    let mut frames = Vec::new();
    let mut asked = 0;
    for _ in 0..120 {
        let start = Instant::now();
        asked += h.cx.update(|window, cx| window.simulate_next_frame(cx));
        h.cx.update(|window, cx| window.draw(cx).clear(cx));
        frames.push(start.elapsed());
    }
    let mean = frames.iter().sum::<Duration>() / frames.len() as u32;
    eprintln!(
        "{name}: animation callbacks={asked}/120 frames, mean frame={mean:?}, at 60 fps={:?}/s",
        if asked > 0 { mean * 60 } else { Duration::ZERO }
    );
}

#[gpui_kit::test]
#[ignore]
fn perf_idle_window(cx: &mut TestAppContext) {
    let mut h = harness(cx, threads());
    animation_frames(&mut h, "idle");
}

#[gpui_kit::test]
#[ignore]
fn perf_idle_window_with_an_unseen_thread(cx: &mut TestAppContext) {
    let mut h = harness(cx, threads());
    h.workspace.update(&mut h.cx, |workspace, cx| {
        workspace.unseen.insert("x0".into());
        cx.notify();
    });
    redraw(&mut h.cx);
    animation_frames(&mut h, "unseen thread");
    timed_frames(&mut h, "unseen thread");
}

/// A second of frames that timers ask for, as the clock moves on.
fn timed_frames(h: &mut Harness, name: &str) {
    const STEP: Duration = Duration::from_millis(1);
    let start = Instant::now();
    for _ in 0..1000 {
        h.cx.executor().advance_clock(STEP);
        h.cx.run_until_parked();
    }
    eprintln!(
        "{name}: a second of timed frames cost {:?}",
        start.elapsed()
    );
}

#[gpui_kit::test]
#[ignore]
fn perf_window_while_a_tool_runs(cx: &mut TestAppContext) {
    let mut h = harness(cx, threads());
    send(
        &mut h,
        Event::TurnStarted {
            thread_id: "t".into(),
            turn_id: "turn".into(),
            at: NOW.into(),
            model: None,
        },
    );
    send(
        &mut h,
        Event::ItemAdded {
            item: user("u-live".into(), "Go on".into()),
        },
    );
    send(
        &mut h,
        Event::ItemAdded {
            item: call("c-live".into(), None),
        },
    );
    animation_frames(&mut h, "tool running");
    timed_frames(&mut h, "tool running");
}

#[gpui_kit::test]
#[ignore]
fn perf_window_streaming(cx: &mut TestAppContext) {
    stream_reply(cx, 30);
}

#[gpui_kit::test]
#[ignore]
fn perf_window_streaming_a_long_reply(cx: &mut TestAppContext) {
    stream_reply(cx, 200);
}

/// Streams a reply of `paragraphs` into the open thread, 60 characters at a time.
fn stream_reply(cx: &mut TestAppContext, paragraphs: usize) {
    let mut h = harness(cx, threads());
    send(
        &mut h,
        Event::TurnStarted {
            thread_id: "t".into(),
            turn_id: "turn".into(),
            at: NOW.into(),
            model: None,
        },
    );
    let reply = answer_text(99, paragraphs);
    let chars: Vec<char> = reply.chars().collect();
    let mut batches = Vec::new();
    for chunk in chars.chunks(60) {
        let start = Instant::now();
        (h.notify)(Notification::Stream(vec![StreamDelta {
            thread_id: "t".into(),
            item_id: "live".into(),
            kind: StreamKind::Text,
            delta: chunk.iter().collect(),
        }]));
        h.cx.run_until_parked();
        batches.push(start.elapsed());
    }
    let last = &batches[batches.len() - batches.len() / 10..];
    let last_mean = last.iter().sum::<Duration>() / last.len() as u32;
    let mean = batches.iter().sum::<Duration>() / batches.len() as u32;
    batches.sort();
    eprintln!(
        "streaming {} bytes: batches={} mean={mean:?} p90={:?} last tenth mean={last_mean:?}",
        reply.len(),
        batches.len(),
        batches[batches.len() * 9 / 10]
    );
}
