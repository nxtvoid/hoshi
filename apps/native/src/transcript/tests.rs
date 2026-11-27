use super::TranscriptView;
use crate::state::ThreadState;
use gpui_kit::base::TextSelection;
use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{
    AppContext as _, Bounds, ClipboardItem, Entity, Modifiers, MouseButton, Pixels, Point,
    TestAppContext, VisualTestContext, WindowBounds, WindowOptions, point, px, size,
};
use hoshi_core::{ItemContent, ThreadItem, ToolDetail};

fn message(index: usize) -> ThreadItem {
    ThreadItem {
        id: format!("m{index}"),
        thread_id: "t".into(),
        created_at: "2026-11-12T10:00:00Z".into(),
        content: ItemContent::AssistantMessage {
            text: format!("Message number {index}, with **bold** and `code`."),
        },
    }
}

fn open(cx: &mut TestAppContext, count: usize) -> VisualTestContext {
    open_items(cx, 0..count).0
}

fn open_items(
    cx: &mut TestAppContext,
    items: std::ops::Range<usize>,
) -> (
    VisualTestContext,
    Entity<ThreadState>,
    Entity<TranscriptView>,
) {
    open_with(cx, items.map(message).collect())
}

fn open_with(
    cx: &mut TestAppContext,
    items: Vec<ThreadItem>,
) -> (
    VisualTestContext,
    Entity<ThreadState>,
    Entity<TranscriptView>,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        hoshi_ui::theme::apply(ThemeMode::Dark, cx);
    });
    let (window, thread, view) = cx.update(|cx| {
        let thread = cx.new(|_| ThreadState::new("t", items));
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(800.), px(400.)),
            })),
            ..Default::default()
        };
        let shown = thread.clone();
        let (window, view) = gpui_kit::open_window(options, cx, |_, cx| {
            cx.new(|cx| TranscriptView::new(shown, false, cx))
        })
        .expect("test window");
        (window, thread, view)
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.run_until_parked();
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
    (cx, thread, view)
}

fn drag(cx: &mut VisualTestContext, from: Point<gpui_kit::Pixels>, to: Point<gpui_kit::Pixels>) {
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

/// A point on the text of the answer `selector` names.
fn text_of(cx: &mut VisualTestContext, selector: &'static str) -> Point<gpui_kit::Pixels> {
    let answer = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not on screen"));
    point(px(40.), answer.top() + px(8.))
}

fn selected(cx: &mut VisualTestContext) -> String {
    cx.update(TextSelection::selected_text)
}

#[gpui_kit::test]
fn a_drag_selects_across_messages(cx: &mut TestAppContext) {
    let mut cx = open(cx, 6);
    drag(&mut cx, point(px(40.), px(10.)), point(px(760.), px(370.)));

    let text = selected(&mut cx);
    let messages = (0..6)
        .filter(|index| text.contains(&format!("Message number {index}")))
        .count();
    assert!(messages >= 3, "selected: {text:?}");
    assert!(text.contains('\n'), "selected: {text:?}");

    cx.update(|_, cx| cx.write_to_clipboard(ClipboardItem::new_string("before".into())));
    let copy = if cfg!(target_os = "macos") {
        "cmd-c"
    } else {
        "ctrl-c"
    };
    cx.simulate_keystrokes(copy);
    cx.run_until_parked();
    let copied = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(copied.as_deref().map(str::trim), Some(text.trim()));
    assert!(
        text.contains("**bold**") && text.contains("`code`"),
        "{text:?}"
    );
}

#[gpui_kit::test]
fn a_copy_across_messages_keeps_their_markdown_and_links(cx: &mut TestAppContext) {
    let linked = |index: usize| ThreadItem {
        id: format!("l{index}"),
        thread_id: "t".into(),
        created_at: "2026-11-12T10:00:00Z".into(),
        content: ItemContent::AssistantMessage {
            text: format!("Step {index}: see [the repo](https://github.com/a/b) and _this_."),
        },
    };
    let (mut cx, _, _) = open_with(cx, (0..4).map(linked).collect());
    drag(&mut cx, point(px(40.), px(30.)), point(px(760.), px(370.)));
    let copy = if cfg!(target_os = "macos") {
        "cmd-c"
    } else {
        "ctrl-c"
    };
    cx.simulate_keystrokes(copy);
    cx.run_until_parked();
    let copied = cx
        .update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
        .unwrap_or_default();
    assert!(
        copied.contains("[the repo](https://github.com/a/b)") && copied.contains("*this*"),
        "{copied:?}"
    );
    assert!(!copied.contains("hoshi-site"), "{copied:?}");
    assert!(!copied.contains('\u{a0}'), "{copied:?}");
}

#[gpui_kit::test]
fn a_long_transcript_stays_virtual(cx: &mut TestAppContext) {
    let mut cx = open(cx, 2_000);
    cx.run_until_parked();
    for _ in 0..5 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
    let start = text_of(&mut cx, "answer-m1997");
    drag(&mut cx, start, point(px(760.), px(390.)));

    let text = selected(&mut cx);
    assert!(text.contains("Message number 1998"), "selected: {text:?}");
    assert!(!text.contains("Message number 0,"), "selected: {text:?}");
}

#[gpui_kit::test]
fn an_older_page_goes_above_without_moving_the_view(cx: &mut TestAppContext) {
    let (mut cx, thread, view) = open_items(cx, 100..200);
    thread.update(&mut cx, |thread, cx| {
        let mut older: Vec<_> = (0..100).map(message).collect();
        older.append(&mut thread.items);
        thread.items = older;
        cx.notify();
    });
    cx.run_until_parked();
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    let rows = view.read_with(&cx, |view, cx| view.scroller.read(cx).item_count());
    assert_eq!(rows, 200);
    let start = text_of(&mut cx, "answer-m197");
    drag(&mut cx, start, point(px(760.), px(390.)));
    let text = selected(&mut cx);
    assert!(text.contains("Message number 198"), "selected: {text:?}");
}

const DIFF: &str = "@@ -1,3 +1,3 @@\n fn main() {\n-    old();\n+    new();\n }";

fn call(id: &str, name: &str, input: &str, detail: Option<ToolDetail>) -> ThreadItem {
    ThreadItem {
        id: id.into(),
        thread_id: "t".into(),
        created_at: "2026-11-12T10:00:00Z".into(),
        content: ItemContent::ToolCall {
            name: name.into(),
            input: input.into(),
            output: Some("done".into()),
            is_error: false,
            detail,
            output_omitted: false,
            interrupted: false,
        },
    }
}

fn edit(id: &str, diff: Option<&str>) -> ThreadItem {
    call(
        id,
        "Edit",
        r#"{"file_path":"src/main.rs"}"#,
        Some(ToolDetail::FileChange {
            path: "src/main.rs".into(),
            additions: 1,
            deletions: 1,
            diff: diff.map(str::to_owned),
        }),
    )
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not on screen"));
    cx.simulate_click(bounds.center(), Modifiers::default());
    redraw(cx);
}

#[gpui_kit::test]
fn a_subagent_shows_as_a_card_that_opens_it(cx: &mut TestAppContext) {
    let subagent = call(
        "s1",
        "Agent",
        r#"{"description":"Wait and say hola","prompt":"Wait"}"#,
        None,
    );
    let (mut cx, _, view) = open_with(cx, vec![message(0), subagent]);
    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = opened.clone();
    cx.update(|_, cx| {
        cx.subscribe(
            &view,
            move |_, event: &crate::subagents::OpenSubagent, _| {
                seen.borrow_mut().push(event.0.clone())
            },
        )
        .detach();
    });
    assert!(cx.debug_bounds("disclosure-s1").is_none());
    click(&mut cx, "subagent-s1");
    assert_eq!(*opened.borrow(), ["s1"]);
}

#[gpui_kit::test]
fn a_file_change_opens_to_its_diff_and_closes(cx: &mut TestAppContext) {
    let (mut cx, _, _) = open_with(cx, vec![message(0), edit("e1", Some(DIFF)), message(1)]);
    assert!(cx.debug_bounds("diff-view").is_none());

    click(&mut cx, "disclosure-e1");
    let diff = cx.debug_bounds("diff-view").expect("the diff shows");
    assert!(diff.size.height > px(40.), "diff: {diff:?}");

    click(&mut cx, "disclosure-e1");
    assert!(cx.debug_bounds("diff-view").is_none());
}

#[gpui_kit::test]
fn a_diff_the_page_left_out_shows_once_read_and_pushes_the_rows_below(cx: &mut TestAppContext) {
    let (mut cx, thread, _) = open_with(
        cx,
        vec![
            edit("e1", None),
            message(0),
            call("g1", "Grep", r#"{"pattern":"main"}"#, None),
        ],
    );
    click(&mut cx, "disclosure-e1");
    assert!(cx.debug_bounds("diff-view").is_none());

    thread.update(&mut cx, |thread, cx| {
        let whole = edit("e1", Some(DIFF)).content;
        thread.whole.insert("e1".into(), whole);
        cx.notify();
    });
    redraw(&mut cx);

    let diff = cx.debug_bounds("diff-view").expect("the diff shows");
    let below = cx
        .debug_bounds("disclosure-g1")
        .expect("the next call shows");
    assert!(
        below.top() >= diff.bottom(),
        "diff: {diff:?}, below: {below:?}"
    );
}

#[gpui_kit::test]
fn a_run_of_calls_opens_to_each_call(cx: &mut TestAppContext) {
    let (mut cx, _, _) = open_with(
        cx,
        vec![
            message(0),
            call("g1", "Grep", r#"{"pattern":"main"}"#, None),
            call("r1", "Read", r#"{"file_path":"a.rs"}"#, None),
            call("r2", "Read", r#"{"file_path":"b.rs"}"#, None),
            message(1),
        ],
    );
    assert!(cx.debug_bounds("disclosure-r1").is_none());

    click(&mut cx, "disclosure-work:g1");
    assert!(cx.debug_bounds("disclosure-r1").is_some());
    assert!(cx.debug_bounds("disclosure-r2").is_some());
}

#[gpui_kit::test]
fn every_open_call_of_a_run_shows_its_whole_body(cx: &mut TestAppContext) {
    let long = (0..40)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut items = vec![message(0)];
    for index in 0..4 {
        let mut item = call(
            &format!("c{index}"),
            "Bash",
            &format!(r#"{{"command":"echo {index}"}}"#),
            None,
        );
        if let ItemContent::ToolCall { output, .. } = &mut item.content {
            *output = Some(long.clone());
        }
        items.push(item);
    }
    items.push(message(1));
    let (mut cx, thread, view) = open_with(cx, items);
    view.update(&mut cx, |view, cx| {
        view.toggle("work:c0", cx);
        for index in 0..4 {
            view.toggle(&format!("c{index}"), cx);
        }
    });
    redraw(&mut cx);
    thread.update(&mut cx, |thread, cx| {
        let mut whole = thread.items[2].content.clone();
        if let ItemContent::ToolCall { output, .. } = &mut whole {
            *output = Some([long.as_str(); 3].join("\n"));
        }
        thread.whole.insert("c1".into(), whole);
        cx.notify();
    });
    redraw(&mut cx);

    for block in ["code-block-c2:output", "code-block-c3:output"] {
        let bounds = cx
            .debug_bounds(block)
            .unwrap_or_else(|| panic!("{block} is not on screen"));
        assert!(bounds.size.height > px(100.), "{block}: {bounds:?}");
    }
    let output = cx.debug_bounds("code-block-c2:output").unwrap();
    let below = cx.debug_bounds("disclosure-c3").unwrap();
    assert!(below.top() >= output.bottom(), "{output:?} / {below:?}");
}

fn user(id: &str, created_at: &str) -> ThreadItem {
    ThreadItem {
        id: id.into(),
        thread_id: "t".into(),
        created_at: created_at.into(),
        content: ItemContent::UserMessage {
            text: format!("Question {id}"),
            attachments: Vec::new(),
            steer: false,
            task_result: None,
            pull_request_update: None,
            resumed: None,
            rewind: None,
            dropped: false,
            model_switch: None,
        },
    }
}

#[gpui_kit::test]
fn a_user_message_rewraps_when_the_column_widens(cx: &mut TestAppContext) {
    let long = ThreadItem {
        content: ItemContent::UserMessage {
            text: "A question long enough to wrap in a narrow column, ".repeat(6),
            attachments: Vec::new(),
            steer: false,
            task_result: None,
            pull_request_update: None,
            resumed: None,
            rewind: None,
            dropped: false,
            model_switch: None,
        },
        ..user("u1", "2026-11-12T10:00:00Z")
    };
    let (mut cx, _, _) = open_with(cx, vec![long, user("u2", "2026-11-12T10:01:00Z")]);
    let bubble = |cx: &mut VisualTestContext| {
        redraw(cx);
        cx.debug_bounds("collapse-u1").expect("the bubble shows")
    };
    let wide = bubble(&mut cx);
    let short = cx
        .debug_bounds("collapse-u2")
        .expect("the short bubble shows");
    assert!(
        short.size.width < wide.size.width / 2.,
        "{short:?} against {wide:?}"
    );
    cx.simulate_resize(size(px(400.), px(400.)));
    let narrow = bubble(&mut cx);
    assert!(
        narrow.size.width < wide.size.width,
        "{narrow:?} against {wide:?}"
    );
    assert!(
        narrow.size.height > wide.size.height,
        "{narrow:?} against {wide:?}"
    );
    cx.simulate_resize(size(px(800.), px(400.)));
    let widened = bubble(&mut cx);
    assert_eq!(widened.size, wide.size, "{widened:?} against {wide:?}");
}

fn answer(id: &str, created_at: &str) -> ThreadItem {
    ThreadItem {
        id: id.into(),
        thread_id: "t".into(),
        created_at: created_at.into(),
        content: ItemContent::AssistantMessage {
            text: format!("Answer {id}"),
        },
    }
}

fn answered_turn() -> Vec<ThreadItem> {
    vec![
        user("u1", "2026-11-12T10:00:00Z"),
        call("g1", "Grep", r#"{"pattern":"main"}"#, None),
        call("r1", "Read", r#"{"file_path":"a.rs"}"#, None),
        answer("a1", "2026-11-12T10:02:05Z"),
    ]
}

#[gpui_kit::test]
fn an_answered_turn_folds_its_work_and_opens_on_toggle(cx: &mut TestAppContext) {
    let (mut cx, _, view) = open_with(cx, answered_turn());
    let rows = view.read_with(&cx, |view, cx| view.scroller.read(cx).item_count());
    assert_eq!(rows, 3);
    assert!(cx.debug_bounds("fold-row-fold:u1").is_some());
    assert!(cx.debug_bounds("disclosure-work:g1").is_none());

    click(&mut cx, "fold-row-fold:u1");
    assert!(cx.debug_bounds("disclosure-work:g1").is_some());
    click(&mut cx, "disclosure-work:g1");
    assert!(cx.debug_bounds("disclosure-r1").is_some());

    click(&mut cx, "fold-row-fold:u1");
    assert!(cx.debug_bounds("disclosure-work:g1").is_none());
}

#[test]
fn a_waited_background_task_shows_its_end_and_the_status_follows_it() {
    let items = vec![
        user("u1", "2026-11-12T10:00:00Z"),
        call(
            "b1",
            "Bash",
            r#"{"command":"sleep 20","run_in_background":true}"#,
            None,
        ),
        answer("a1", "2026-11-12T10:00:07Z"),
        ThreadItem {
            id: "e1".into(),
            thread_id: "t".into(),
            created_at: "2026-11-12T10:00:08Z".into(),
            content: ItemContent::RunEnded,
        },
        ThreadItem {
            id: "t1".into(),
            thread_id: "t".into(),
            created_at: "2026-11-12T10:00:27Z".into(),
            content: ItemContent::BackgroundTaskEnded {
                tool_call_id: Some("b1".into()),
                failed: false,
            },
        },
    ];
    let rows = super::shown_rows(
        crate::fold::fold_turns(super::group(&items), &items, true, &|_| false),
        &items,
    );
    let ended = rows
        .iter()
        .position(|row| *row == super::Row::Item(4))
        .expect("the task's end is a row");
    assert_eq!(super::status_anchor(&rows, &items), (ended + 1, Some(4)));
}

#[gpui_kit::test]
fn a_turn_still_at_work_stays_open_until_it_ends(cx: &mut TestAppContext) {
    let (mut cx, _, view) = open_with(cx, answered_turn());
    view.update(&mut cx, |view, cx| view.set_working(true, cx));
    redraw(&mut cx);
    assert!(cx.debug_bounds("fold-row-fold:u1").is_none());
    assert!(cx.debug_bounds("disclosure-work:g1").is_some());
    let rows = view.read_with(&cx, |view, cx| view.scroller.read(cx).item_count());
    assert_eq!(rows, 4, "the turn's status joins its rows");

    view.update(&mut cx, |view, cx| view.set_working(false, cx));
    redraw(&mut cx);
    assert!(cx.debug_bounds("fold-row-fold:u1").is_some());
    assert!(cx.debug_bounds("disclosure-work:g1").is_none());
}

#[gpui_kit::test]
fn an_older_page_that_starts_a_turn_folds_it_with_the_rows_below(cx: &mut TestAppContext) {
    let turn = answered_turn();
    let (mut cx, thread, view) = open_with(
        cx,
        [&turn[2..], &[user("u2", "2026-11-12T10:05:00Z")]].concat(),
    );
    let rows = view.read_with(&cx, |view, cx| view.scroller.read(cx).item_count());
    assert_eq!(rows, 3);

    thread.update(&mut cx, |thread, cx| {
        let mut older = turn[..2].to_vec();
        older.append(&mut thread.items);
        thread.items = older;
        cx.notify();
    });
    redraw(&mut cx);

    let rows = view.read_with(&cx, |view, cx| view.scroller.read(cx).item_count());
    assert_eq!(rows, 4);
    assert!(cx.debug_bounds("fold-row-fold:u1").is_some());
}

#[test]
fn rows_put_above_or_added_below_replace_only_what_changed() {
    use super::{Row, changed_rows};
    let old = [Row::Work(vec![0, 1]), Row::Item(2), Row::Item(3)];
    let prepended = [Row::Item(0), Row::Item(1), Row::Item(4), Row::Item(5)];
    assert_eq!(changed_rows(&old, &prepended, 2), (0..1, 2));
    let appended = [
        Row::Work(vec![0, 1]),
        Row::Item(2),
        Row::Item(3),
        Row::Item(4),
    ];
    assert_eq!(changed_rows(&old, &appended, 0), (3..3, 1));
    let grown = [Row::Work(vec![0, 1, 2]), Row::Item(3)];
    assert_eq!(
        changed_rows(&[Row::Work(vec![0, 1]), Row::Item(3)], &grown, 0),
        (0..1, 1)
    );
    let streaming = [Row::Item(0), Row::Streaming(0)];
    assert_eq!(changed_rows(&streaming, &streaming, 0), (2..2, 0));
}

/// Where `needle` is drawn, when it shows.
fn drawn_at(cx: &mut VisualTestContext, needle: &str) -> Option<Point<gpui_kit::Pixels>> {
    cx.update(|window, cx| {
        gpui_kit::base::test_support::begin_frame(window);
        window.refresh();
        window.draw(cx).clear(cx);
    });
    cx.update(|window, _| gpui_kit::base::test_support::texts(window))
        .into_iter()
        .find(|text| text.visible() && text.text().contains(needle))
        .map(|text| text.bounds().origin + point(px(2.), text.bounds().size.height / 2.))
}

#[gpui_kit::test]
fn a_selection_through_a_long_answer_copies_the_blocks_scrolled_past(cx: &mut TestAppContext) {
    let text = (0..60)
        .map(|index| format!("Paragraph {index} says word{index}."))
        .collect::<Vec<_>>()
        .join("\n\n");
    let answer = ThreadItem {
        content: ItemContent::AssistantMessage { text },
        ..message(0)
    };
    let (mut cx, _, _) = open_with(cx, vec![answer]);
    let start = drawn_at(&mut cx, "Paragraph 57").expect("the answer's end shows");
    assert!(
        drawn_at(&mut cx, "Paragraph 20").is_none(),
        "far up is out of view"
    );

    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    for _ in 0..12 {
        cx.simulate_event(gpui_kit::ScrollEvent {
            position: start,
            delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(80.))),
            ..Default::default()
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
    let end = drawn_at(&mut cx, "Paragraph 20").expect("scrolled up to it");
    assert!(
        drawn_at(&mut cx, "Paragraph 57").is_none(),
        "the start scrolled away"
    );
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));

    let copied = selected(&mut cx);
    for index in 21..57 {
        assert!(
            copied.contains(&format!("Paragraph {index} says word{index}.")),
            "paragraph {index} missing: {copied:?}"
        );
    }
    assert!(copied.contains("Paragraph 57"), "{copied:?}");
    assert!(!copied.contains("word58"), "{copied:?}");
    assert!(!copied.contains("Paragraph 19"), "{copied:?}");
    assert!(copied.contains("\n\n"), "paragraphs stay apart: {copied:?}");
}

#[gpui_kit::test]
fn a_long_answer_keeps_its_height_as_it_scrolls_past(cx: &mut TestAppContext) {
    let text = (0..40)
        .map(|index| match index % 4 {
            0 => format!("Paragraph {index} with `code` and **bold**."),
            1 => format!("```rust\nfn step_{index}() {{}}\n```"),
            2 => format!("| a | b |\n| - | - |\n| {index} | x |"),
            _ => format!("- item {index}\n- another"),
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let answer = ThreadItem {
        content: ItemContent::AssistantMessage { text },
        ..message(0)
    };
    let (mut cx, _, _) = open_with(cx, vec![answer]);
    let height = |cx: &mut VisualTestContext| {
        cx.debug_bounds("answer-m0")
            .expect("the answer shows")
            .size
            .height
    };
    let whole = height(&mut cx);
    for step in 0..30 {
        cx.simulate_event(gpui_kit::ScrollEvent {
            position: point(px(400.), px(200.)),
            delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(120.))),
            ..Default::default()
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(height(&mut cx), whole, "after {step} steps up");
    }
}

fn long_answer(id: &str, created_at: &str) -> ThreadItem {
    let text = (0..60)
        .map(|index| format!("Paragraph {index} says word{index}."))
        .collect::<Vec<_>>()
        .join("\n\n");
    ThreadItem {
        content: ItemContent::AssistantMessage { text },
        ..answer(id, created_at)
    }
}

fn question(id: &str, created_at: &str, text: String) -> ThreadItem {
    let mut item = user(id, created_at);
    if let ItemContent::UserMessage { text: question, .. } = &mut item.content {
        *question = text;
    }
    item
}

fn scroll_top(cx: &mut VisualTestContext, view: &Entity<TranscriptView>) -> (usize, Pixels) {
    view.read_with(cx, |view, cx| {
        let top = view.scroller.read(cx).logical_scroll_top();
        (top.item_ix, top.offset_in_item)
    })
}

/// Turns the wheel `steps` times, up for a positive `delta`, and fails if the view moves the
/// other way.
fn scroll_steadily(
    cx: &mut VisualTestContext,
    view: &Entity<TranscriptView>,
    delta: f32,
    steps: usize,
) {
    for step in 0..steps {
        let before = scroll_top(cx, view);
        cx.simulate_event(gpui_kit::ScrollEvent {
            position: point(px(200.), px(200.)),
            delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(delta))),
            ..Default::default()
        });
        for _ in 0..2 {
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
        let after = scroll_top(cx, view);
        let steady = if delta > 0. {
            after <= before
        } else {
            after >= before
        };
        assert!(steady, "step {step}: {before:?} to {after:?}");
    }
}

#[gpui_kit::test]
fn scrolling_past_a_long_message_never_jumps_back(cx: &mut TestAppContext) {
    let (mut cx, _, view) = open_with(
        cx,
        vec![
            long_answer("a0", "2026-11-12T10:00:00Z"),
            question(
                "u1",
                "2026-11-12T10:01:00Z",
                "A long question.\n\n".repeat(40),
            ),
            long_answer("a1", "2026-11-12T10:02:00Z"),
        ],
    );
    scroll_steadily(&mut cx, &view, 80., 40);
    scroll_steadily(&mut cx, &view, -80., 40);
    scroll_steadily(&mut cx, &view, 80., 40);
}

#[gpui_kit::test]
fn a_message_that_grows_long_in_a_narrower_column_never_jumps(cx: &mut TestAppContext) {
    let (mut cx, _, view) = open_with(
        cx,
        vec![
            question(
                "u1",
                "2026-11-12T10:00:00Z",
                "A question long enough to wrap in a narrow column, ".repeat(16),
            ),
            long_answer("a1", "2026-11-12T10:01:00Z"),
        ],
    );
    scroll_steadily(&mut cx, &view, 80., 40);
    assert!(
        cx.debug_bounds("collapse-u1-toggle").is_none(),
        "whole in the wide column"
    );
    scroll_steadily(&mut cx, &view, -80., 40);
    cx.simulate_resize(size(px(360.), px(400.)));
    redraw(&mut cx);
    scroll_steadily(&mut cx, &view, 80., 60);
    assert!(
        cx.debug_bounds("collapse-u1-toggle").is_some(),
        "collapsed in the narrow column"
    );
}
