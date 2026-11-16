use super::ThreadDetails;
use crate::state::{self, Workspace};
use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, IntoElement, Modifiers, ParentElement as _, Point,
    Render, Styled as _, TestAppContext, VisualTestContext, Window, WindowBounds, WindowOptions,
    div, px, size,
};
use hoshi_core::{AccessMode, Command, Editor, GitAction, GitActionRequest, ThreadWorkspace};
use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Chat {
    details: Entity<ThreadDetails>,
}

impl Render for Chat {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let toggle = self
            .details
            .update(cx, |details, cx| details.header_toggle(cx));
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().h_10().flex().child(toggle))
            .child(div().relative().flex_1().child(self.details.clone()))
    }
}

struct Harness {
    cx: VisualTestContext,
    details: Entity<ThreadDetails>,
}

fn git(dir: &Path, args: &[&str]) {
    let status = Process::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?}");
}

fn repository(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hoshi-native-{name}-{}", uuid::Uuid::new_v4()));
    let project = dir.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(dir.join("home")).unwrap();
    git(&project, &["init", "-q", "-b", "main"]);
    git(&project, &["config", "user.name", "hoshi"]);
    git(&project, &["config", "user.email", "hoshi@example.com"]);
    std::fs::write(project.join("notes.txt"), "one\ntwo\n").unwrap();
    git(&project, &["add", "."]);
    git(&project, &["commit", "-q", "-m", "first"]);
    git(&project, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(project.join("notes.txt"), "one\nthree\nfour\n").unwrap();
    dir
}

fn harness(cx: &mut TestAppContext, name: &str, width: f32) -> Harness {
    cx.executor().allow_parking();
    let dir = repository(name);
    let (notify, notifications) = state::channel();
    let engine = Arc::new(hoshi_engine::Engine::open(&dir.join("home"), notify).unwrap());
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
            title: "thread".into(),
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
    });
    let workspace = cx.update(|cx| cx.new(|cx| Workspace::new(engine, notifications, cx)));
    workspace.update(cx, |workspace, cx| workspace.open_thread("t", cx));
    let (window, details) = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(width), px(600.)),
            })),
            ..Default::default()
        };
        let mut details = None;
        let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
            let shown = cx.new(|cx| ThreadDetails::new(workspace.clone(), window, cx));
            details = Some(shown.clone());
            cx.new(|_| Chat { details: shown })
        })
        .unwrap();
        (window, details.unwrap())
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    redraw(&mut cx);
    Harness { cx, details }
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
    }
}

fn wait_for(h: &mut Harness, what: &str, done: impl Fn(&ThreadDetails) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        redraw(&mut h.cx);
        if h.details.read_with(&h.cx, |details, _| done(details)) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[gpui_kit::test]
fn the_card_shows_the_checkout_of_a_wide_chat(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-card", 1200.);
    assert!(h.cx.debug_bounds("thread-details-card").is_none());

    h.details
        .update(&mut h.cx, |details, cx| details.toggle(cx));
    wait_for(&mut h, "the checkout", |details| {
        details.changes.contains_key("t") && details.statuses.contains_key("t")
    });

    let card =
        h.cx.debug_bounds("thread-details-card")
            .expect("the card shows");
    assert_eq!(card.size.width, px(super::CARD_WIDTH));
    assert!(card.origin.x > px(800.), "card: {card:?}");
    h.details.read_with(&h.cx, |details, _| {
        assert!(details.is_inline());
        let changes = &details.changes["t"];
        assert_eq!(
            (changes.files, changes.additions, changes.deletions),
            (1, 2, 1)
        );
        let status = &details.statuses["t"];
        assert_eq!(status.branch.as_deref(), Some("feature"));
        assert!(status.has_changes);
    });
}

#[gpui_kit::test]
fn the_editor_menu_drops_under_its_whole_row(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-editor-menu", 1200.);
    h.details.update(&mut h.cx, |details, cx| {
        details.editors = Some(vec![Editor::Vscode, Editor::FileManager]);
        details.toggle(cx);
    });
    redraw(&mut h.cx);

    let row =
        h.cx.debug_bounds("details-editor-row")
            .expect("the editor row shows");
    let arrow =
        h.cx.debug_bounds("details-editor-arrow")
            .expect("the arrow shows");
    h.cx.simulate_click(arrow.center(), Modifiers::default());
    redraw(&mut h.cx);

    let menu = h.cx.debug_bounds("popover-popup").expect("the menu opens");
    assert_eq!(menu.origin.x, row.origin.x, "menu: {menu:?}, row: {row:?}");
    assert_eq!(
        menu.size.width, row.size.width,
        "menu: {menu:?}, row: {row:?}"
    );
    assert!(
        menu.origin.y >= row.bottom(),
        "menu: {menu:?}, row: {row:?}"
    );
}

#[gpui_kit::test]
fn a_narrow_chat_shows_the_details_from_a_popover(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-narrow", 700.);
    h.details
        .update(&mut h.cx, |details, cx| details.toggle(cx));
    redraw(&mut h.cx);

    assert!(!h.details.read_with(&h.cx, |details, _| details.is_inline()));
    assert!(h.cx.debug_bounds("thread-details-card").is_none());

    h.details.update(&mut h.cx, |details, cx| {
        details.popover_open = true;
        cx.notify();
    });
    wait_for(&mut h, "the checkout", |details| {
        details.changes.contains_key("t")
    });
    assert!(h.cx.debug_bounds("popover-popup").is_some());
}

#[gpui_kit::test]
fn the_git_action_commits_and_says_so_in_place_of_the_button(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-commit", 1200.);
    h.details
        .update(&mut h.cx, |details, cx| details.toggle(cx));
    wait_for(&mut h, "the checkout", |details| {
        details.statuses.contains_key("t")
    });
    let quick = h.details.read_with(&h.cx, |details, _| {
        super::git::quick_action(&details.statuses["t"], false)
    });
    assert_eq!(quick.action, GitAction::Commit);

    let request = GitActionRequest {
        message: Some("Note three and four".into()),
        ..h.details
            .read_with(&h.cx, |details, _| details.quick_request(quick.action))
    };
    h.details.update_in(&mut h.cx, |details, window, cx| {
        details.run_git("t".into(), request, window, cx)
    });
    wait_for(&mut h, "the commit", |details| {
        details.done.contains_key("t")
    });
    wait_for(&mut h, "a clean checkout", |details| {
        details
            .statuses
            .get("t")
            .is_some_and(|status| !status.has_changes)
    });

    h.details.read_with(&h.cx, |details, _| {
        let done = &details.done["t"];
        assert!(done.commit.is_some());
        assert_eq!(done.subject.as_deref(), Some("Note three and four"));
        assert!(details.running.is_empty());
    });
}

#[gpui_kit::test]
fn a_disabled_git_action_says_why_on_hover(cx: &mut TestAppContext) {
    let mut h = harness(cx, "details-git-why", 1200.);
    h.details
        .update(&mut h.cx, |details, cx| details.toggle(cx));
    wait_for(&mut h, "the checkout", |details| {
        details.statuses.contains_key("t")
    });
    let arrow =
        h.cx.debug_bounds("details-git-arrow")
            .expect("the git menu's arrow shows");
    h.cx.simulate_click(arrow.center(), Modifiers::default());
    redraw(&mut h.cx);

    let push =
        h.cx.debug_bounds("menu-item-details-git-push")
            .expect("the push item shows");
    h.cx.simulate_mouse_move(push.center(), None, Modifiers::default());
    redraw(&mut h.cx);
    h.cx.executor().advance_clock(Duration::from_millis(700));
    redraw(&mut h.cx);
    let tooltip = h.cx.debug_bounds("tooltip").expect("the reason shows");
    assert!(
        tooltip.right() <= push.left(),
        "tooltip: {tooltip:?}, item: {push:?}"
    );
}

#[gpui_kit::test]
fn a_subagent_row_asks_to_show_its_call_in_the_transcript(cx: &mut TestAppContext) {
    use super::DetailsEvent;
    use hoshi_core::{ItemContent, ThreadItem};
    use std::cell::RefCell;
    use std::rc::Rc;

    let mut h = harness(cx, "details-subagent", 1200.);
    let workspace = h
        .details
        .read_with(&h.cx, |details, _| details.workspace.clone());
    let open = workspace
        .read_with(&h.cx, |workspace, _| workspace.open.clone())
        .expect("the thread is open");
    let deadline = Instant::now() + Duration::from_secs(30);
    while open.read_with(&h.cx, |thread, _| thread.loading) {
        assert!(Instant::now() < deadline, "timed out loading the thread");
        redraw(&mut h.cx);
        std::thread::sleep(Duration::from_millis(20));
    }
    open.update(&mut h.cx, |thread, cx| {
        thread.items.push(ThreadItem {
            id: "call-1".into(),
            thread_id: "t".into(),
            created_at: "2026-11-13T10:00:00Z".into(),
            content: ItemContent::ToolCall {
                name: "Agent".into(),
                input: r#"{"description":"Look around","subagent_type":"Explore"}"#.into(),
                output: None,
                is_error: false,
                detail: None,
                output_omitted: false,
                interrupted: false,
            },
        });
        cx.notify();
    });
    let shown = Rc::new(RefCell::new(Vec::new()));
    let _events = h.cx.update({
        let shown = shown.clone();
        let details = h.details.clone();
        move |_, cx| {
            cx.subscribe(&details, move |_, event: &DetailsEvent, _| {
                shown.borrow_mut().push(event.clone())
            })
        }
    });
    h.details
        .update(&mut h.cx, |details, cx| details.toggle(cx));
    redraw(&mut h.cx);

    let row =
        h.cx.debug_bounds("details-agent-call-1")
            .expect("the subagent's row shows");
    h.cx.simulate_click(row.center(), Modifiers::default());
    redraw(&mut h.cx);

    assert_eq!(
        *shown.borrow(),
        [DetailsEvent::ShowItem {
            thread_id: "t".into(),
            item_id: "call-1".into(),
        }]
    );
}
