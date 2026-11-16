use super::{DEFAULT_WIDTH, MAX_WIDTH, RightPanel, SurfaceKind, SurfaceView};
use crate::state::{self, Workspace};
use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, IntoElement, Modifiers, MouseButton,
    ParentElement as _, Point, Render, Styled as _, TestAppContext, VisualTestContext, Window,
    WindowBounds, WindowOptions, div, point, prelude::FluentBuilder as _, px, size,
};
use hoshi_core::{AccessMode, Command, ThreadWorkspace};
use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::sync::Arc;
use std::time::{Duration, Instant};

const WIDTH: f32 = 1000.;

/// The chat on the left, the panel on the right, as the window lays them out.
struct Shell {
    panel: Entity<RightPanel>,
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let maximized = self.panel.read(cx).is_maximized("t");
        div()
            .size_full()
            .flex()
            .when(!maximized, |this| this.child(div().flex_1()))
            .child(self.panel.clone())
    }
}

struct Harness {
    cx: VisualTestContext,
    panel: Entity<RightPanel>,
    workspace: Entity<Workspace>,
    dir: PathBuf,
}

fn git(dir: &Path, args: &[&str]) {
    let status = Process::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git runs");
    assert!(status.success(), "git {args:?}");
}

fn repository(name: &str, big: usize) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hoshi-native-panel-{name}-{}",
        uuid::Uuid::new_v4()
    ));
    let project = dir.join("project");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(dir.join("home")).unwrap();
    git(&project, &["init", "-q", "-b", "main"]);
    git(&project, &["config", "user.name", "hoshi"]);
    git(&project, &["config", "user.email", "hoshi@example.com"]);
    git(&project, &["config", "core.autocrlf", "false"]);
    std::fs::write(project.join("notes.txt"), "one\ntwo\n").unwrap();
    let lines = |word: &str| {
        (0..big)
            .map(|n| format!("{word} {n}\n"))
            .collect::<String>()
    };
    std::fs::write(project.join("big.txt"), lines("old")).unwrap();
    git(&project, &["add", "."]);
    git(&project, &["commit", "-q", "-m", "first"]);
    std::fs::write(project.join("notes.txt"), "one\nthree\nfour\n").unwrap();
    if big > 0 {
        std::fs::write(project.join("big.txt"), lines("new")).unwrap();
    }
    dir
}

fn harness(cx: &mut TestAppContext, name: &str, big: usize) -> Harness {
    harness_with(cx, name, big, false)
}

/// With `saves`, the panel keeps `panel.json` in the test's home.
fn harness_with(cx: &mut TestAppContext, name: &str, big: usize, saves: bool) -> Harness {
    cx.executor().allow_parking();
    let dir = repository(name, big);
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
        hoshi_ui::DialogLayer::init(cx);
        cx.set_reduce_motion(true);
        hoshi_ui::theme::apply(ThemeMode::Dark, cx);
    });
    let workspace = cx.update(|cx| cx.new(|cx| Workspace::new(engine, notifications, cx)));
    if saves {
        let settings = dir.join("home").join("settings.json");
        workspace.update(cx, |workspace, _| workspace.settings_path = Some(settings));
    }
    workspace.update(cx, |workspace, cx| workspace.open_thread("t", cx));
    let kept = workspace.clone();
    let (window, panel) = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(WIDTH), px(600.)),
            })),
            ..Default::default()
        };
        let mut panel = None;
        let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
            let shown = cx.new(|cx| RightPanel::new(workspace.clone(), window, cx));
            panel = Some(shown.clone());
            cx.new(|_| Shell { panel: shown })
        })
        .unwrap();
        (window, panel.unwrap())
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    redraw(&mut cx);
    Harness {
        cx,
        panel,
        workspace: kept,
        dir,
    }
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
    }
}

fn wait_until(h: &mut Harness, what: &str, done: impl Fn(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        redraw(&mut h.cx);
        if done(&mut h.cx) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_rows(h: &mut Harness, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while rows(h) < count {
        assert!(Instant::now() < deadline, "only {} rows", rows(h));
        redraw(&mut h.cx);
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn click(h: &mut Harness, selector: &'static str) {
    let bounds =
        h.cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not on screen"));
    h.cx.simulate_click(bounds.center(), Modifiers::default());
    redraw(&mut h.cx);
}

fn open(h: &Harness) -> bool {
    h.panel.read_with(&h.cx, |panel, _| panel.is_open("t"))
}

fn kinds(h: &Harness) -> Vec<SurfaceKind> {
    h.panel.read_with(&h.cx, |panel, _| {
        panel
            .surfaces("t")
            .map(|panel| {
                panel
                    .surfaces
                    .iter()
                    .map(|surface| surface.kind())
                    .collect()
            })
            .unwrap_or_default()
    })
}

fn rows(h: &Harness) -> usize {
    h.panel.read_with(&h.cx, |panel, cx| {
        panel
            .views
            .values()
            .find_map(|view| match view {
                SurfaceView::Diff { view, .. } => Some(view.read(cx).files().read(cx).row_count()),
                _ => None,
            })
            .unwrap_or(0)
    })
}

#[gpui_kit::test]
fn the_launcher_opens_the_diff_whose_file_opens_to_its_lines(cx: &mut TestAppContext) {
    let mut h = harness(cx, "diff", 0);
    assert!(h.cx.debug_bounds("right-panel").is_none());

    h.panel.update(&mut h.cx, |panel, cx| panel.toggle("t", cx));
    redraw(&mut h.cx);
    assert!(
        h.cx.debug_bounds("launch-D").is_some(),
        "the launcher shows"
    );

    h.cx.simulate_keystrokes("d");
    wait_until(&mut h, "the changed file", |cx| {
        cx.debug_bounds("changed-file-notes.txt").is_some()
    });
    assert_eq!(kinds(&h), [SurfaceKind::Diff]);
    assert_eq!(rows(&h), 1);

    click(&mut h, "changed-file-notes.txt");
    wait_for_rows(&mut h, 4);

    click(&mut h, "surface-tab-close-0");
    assert!(!open(&h), "closing the last surface closes the panel");
    assert!(kinds(&h).is_empty());
}

#[gpui_kit::test]
fn a_letter_in_the_add_menu_opens_its_surface_at_once(cx: &mut TestAppContext) {
    let mut h = harness(cx, "add-menu", 0);
    h.panel.update(&mut h.cx, |panel, cx| panel.toggle("t", cx));
    redraw(&mut h.cx);
    h.cx.simulate_keystrokes("d");
    wait_until(&mut h, "the diff", |cx| {
        cx.debug_bounds("changed-file-notes.txt").is_some()
    });

    h.panel.update(&mut h.cx, |panel, cx| {
        panel.add_menu_open = true;
        cx.notify();
    });
    redraw(&mut h.cx);
    assert!(
        h.cx.debug_bounds("popover-popup").is_some(),
        "the menu shows"
    );
    h.cx.simulate_keystrokes("b");
    redraw(&mut h.cx);

    assert_eq!(kinds(&h), [SurfaceKind::Diff, SurfaceKind::Browser]);
    assert!(
        h.cx.debug_bounds("popover-popup").is_none(),
        "the menu closed"
    );
}

#[gpui_kit::test]
fn ctrl_alt_b_toggles_and_closing_ends_maximized(cx: &mut TestAppContext) {
    let mut h = harness(cx, "keys", 0);
    h.cx.simulate_keystrokes("ctrl-alt-b");
    redraw(&mut h.cx);
    assert!(open(&h));
    h.panel
        .update(&mut h.cx, |panel, cx| panel.toggle_maximized(cx));
    redraw(&mut h.cx);
    assert!(h.panel.read_with(&h.cx, |panel, _| panel.is_maximized("t")));
    let panel = h.cx.debug_bounds("right-panel").expect("the panel shows");
    assert_eq!(
        panel.size.width,
        px(WIDTH),
        "maximized takes the chat's place"
    );

    h.cx.simulate_keystrokes("ctrl-alt-b");
    redraw(&mut h.cx);
    assert!(!open(&h));
    h.cx.simulate_keystrokes("ctrl-alt-b");
    redraw(&mut h.cx);
    assert!(open(&h));
    assert!(!h.panel.read_with(&h.cx, |panel, _| panel.is_maximized("t")));
}

#[gpui_kit::test]
fn dragging_the_left_edge_resizes_within_range(cx: &mut TestAppContext) {
    let mut h = harness(cx, "resize", 0);
    h.panel.update(&mut h.cx, |panel, cx| panel.toggle("t", cx));
    redraw(&mut h.cx);
    let edge = WIDTH - DEFAULT_WIDTH;
    let y = px(300.);

    h.cx.simulate_mouse_down(point(px(edge), y), MouseButton::Left, Modifiers::default());
    redraw(&mut h.cx);
    for x in [edge - 10., edge - 100.] {
        h.cx.simulate_mouse_move(
            point(px(x), y),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        redraw(&mut h.cx);
    }
    h.cx.simulate_mouse_up(
        point(px(edge - 100.), y),
        MouseButton::Left,
        Modifiers::default(),
    );
    redraw(&mut h.cx);
    let width = h.panel.read_with(&h.cx, |panel, _| panel.width());
    assert!(
        (width - (DEFAULT_WIDTH + 100.)).abs() <= 1.,
        "width {width}"
    );
    let panel = h.cx.debug_bounds("right-panel").expect("the panel shows");
    assert_eq!(panel.size.width, px(width));

    let edge = WIDTH - width;
    h.cx.simulate_mouse_down(point(px(edge), y), MouseButton::Left, Modifiers::default());
    redraw(&mut h.cx);
    for x in [edge - 10., 0.] {
        h.cx.simulate_mouse_move(
            point(px(x), y),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        redraw(&mut h.cx);
    }
    h.cx.simulate_mouse_up(point(px(0.), y), MouseButton::Left, Modifiers::default());
    redraw(&mut h.cx);
    assert_eq!(
        h.panel.read_with(&h.cx, |panel, _| panel.width()),
        MAX_WIDTH
    );
}

fn pull_request() -> hoshi_core::PullRequest {
    hoshi_core::PullRequest {
        forge: hoshi_core::ForgeKind::Github,
        number: 42,
        title: "Port the side panel".into(),
        url: "https://example.com/pull/42".into(),
        author: Some("nxtvoid".into()),
        head_branch: "feature".into(),
        head_sha: None,
        base_branch: "main".into(),
        state: hoshi_core::PullRequestState::Open,
        checks: Some(hoshi_core::ChecksState::Failing),
        unresolved_comments: 1,
        open_tasks: 0,
        changes_requested: false,
        turn: None,
        conflicts: false,
        additions: 10,
        deletions: 2,
        updated_at: "2026-11-12T10:00:00Z".into(),
    }
}

fn detail() -> hoshi_core::PullRequestDetail {
    hoshi_core::PullRequestDetail {
        supports: hoshi_core::PullRequestSupport::all(),
        forge_id: "PR_1".into(),
        repository: "nxtvoid/hoshi".into(),
        mergeability: hoshi_core::Mergeability::Mergeable,
        merge_methods: vec![hoshi_core::MergeMethod::Squash],
        can_edit: true,
        body: "Adds **the panel**.".into(),
        changed_files: 3,
        reviewers: vec![hoshi_core::Reviewer {
            name: "ana".into(),
            state: hoshi_core::ReviewState::Approved,
        }],
        labels: vec![hoshi_core::PullRequestLabel {
            name: "ui".into(),
            color: "0e8a16".into(),
        }],
        checks: vec![hoshi_core::PullRequestCheck {
            name: "ci".into(),
            state: hoshi_core::ChecksState::Failing,
            url: None,
        }],
        review_threads: vec![hoshi_core::ReviewThread {
            id: "r1".into(),
            path: "src/panel.rs".into(),
            line: Some(3),
            resolved: false,
            comments: vec![hoshi_core::ReviewComment {
                author: Some("ana".into()),
                body: "Rename this".into(),
                created_at: "2026-11-12T10:00:00Z".into(),
                url: "https://example.com/c/1".into(),
            }],
        }],
        timeline: vec![hoshi_core::TimelineItem::Commit {
            sha: "abcdef0123".into(),
            message: "feat: panel".into(),
            author: Some("nxtvoid".into()),
            created_at: "2026-11-12T10:00:00Z".into(),
            url: "https://example.com/commit/abcdef0".into(),
        }],
    }
}

#[gpui_kit::test]
fn the_pull_request_surface_shows_once_the_thread_has_one(cx: &mut TestAppContext) {
    let mut h = harness(cx, "pull-request", 0);
    h.panel.update(&mut h.cx, |panel, cx| panel.toggle("t", cx));
    redraw(&mut h.cx);
    h.cx.simulate_keystrokes("p");
    redraw(&mut h.cx);
    assert!(kinds(&h).is_empty(), "no pull request, nothing to open");

    let workspace = h.panel.read_with(&h.cx, |panel, _| panel.workspace.clone());
    workspace.update(&mut h.cx, |workspace, cx| {
        workspace.pull_requests.insert("t".into(), pull_request());
        cx.notify();
    });
    h.cx.simulate_keystrokes("p");
    redraw(&mut h.cx);
    assert_eq!(kinds(&h), [SurfaceKind::PullRequest]);
    assert!(h.cx.debug_bounds("pr-body").is_some(), "the skeleton shows");

    workspace.update(&mut h.cx, |workspace, cx| {
        workspace
            .pull_request_details
            .insert("t".into(), Some(detail()));
        cx.notify();
    });
    redraw(&mut h.cx);
    let surface = h.panel.read_with(&h.cx, |panel, _| {
        panel.views.values().find_map(|view| match view {
            SurfaceView::PullRequest(view) => Some(view.clone()),
            _ => None,
        })
    });
    let surface = surface.expect("the pull request view");
    for tab in [
        super::pull_request::Tab::Timeline,
        super::pull_request::Tab::Code,
        super::pull_request::Tab::Summary,
    ] {
        surface.update(&mut h.cx, |surface, cx| surface.select(tab, cx));
        redraw(&mut h.cx);
        assert_eq!(surface.read_with(&h.cx, |surface, _| surface.tab()), tab);
    }
    assert!(h.cx.debug_bounds("pr-body").is_some());
}

#[gpui_kit::test]
fn a_long_diff_only_builds_the_lines_in_view(cx: &mut TestAppContext) {
    let mut h = harness(cx, "long", 2_000);
    h.panel.update(&mut h.cx, |panel, cx| {
        panel.open_surface("t", SurfaceKind::Diff, cx)
    });
    wait_until(&mut h, "the changed files", |cx| {
        cx.debug_bounds("changed-file-big.txt").is_some()
    });
    click(&mut h, "changed-file-big.txt");
    wait_for_rows(&mut h, 2_500);
    let started = Instant::now();
    redraw(&mut h.cx);
    let frames = started.elapsed() / 3;
    eprintln!("{} rows, {frames:?} a frame", rows(&h));
    assert!(h.cx.debug_bounds("changed-file-big.txt").is_some());
    assert!(
        h.cx.debug_bounds("changed-file-notes.txt").is_none(),
        "the file after the long diff stays unbuilt"
    );
}

fn advance(h: &mut Harness, by: Duration) {
    h.cx.executor().advance_clock(by);
    redraw(&mut h.cx);
}

#[gpui_kit::test]
fn the_panel_comes_back_after_a_restart_and_forgets_deleted_threads(cx: &mut TestAppContext) {
    let mut h = harness_with(cx, "saved", 0, true);
    h.panel.update(&mut h.cx, |panel, cx| {
        panel.open_surface("t", SurfaceKind::Diff, cx);
        panel.open_surface("t", SurfaceKind::Browser, cx);
        panel.set_width(500., cx);
        panel.toggle_maximized(cx);
    });
    advance(&mut h, Duration::from_secs(1));
    let file = h.dir.join("home").join("panel.json");
    let text = std::fs::read_to_string(&file).expect("panel.json is written");
    assert!(text.contains(r#""openThreads":{"t":true}"#), "{text}");
    assert!(text.contains(r#""width":500"#), "{text}");

    let workspace = h.workspace.clone();
    let restored =
        h.cx.update(|window, cx| cx.new(|cx| RightPanel::new(workspace.clone(), window, cx)));
    restored.read_with(&h.cx, |panel, _| {
        assert!(panel.is_open("t"));
        assert!(panel.is_maximized("t"));
        assert_eq!(panel.width(), 500.);
        let surfaces = panel.surfaces("t").expect("the thread's surfaces");
        assert_eq!(
            surfaces
                .surfaces
                .iter()
                .map(|surface| surface.kind())
                .collect::<Vec<_>>(),
            [SurfaceKind::Diff, SurfaceKind::Browser]
        );
        assert_eq!(
            surfaces.active_id,
            surfaces.surfaces.get(1).map(|surface| surface.id.clone())
        );
    });

    let engine = h
        .workspace
        .read_with(&h.cx, |workspace, _| workspace.engine.clone());
    engine
        .dispatch(Command::ThreadDelete {
            thread_id: "t".into(),
        })
        .unwrap();
    wait_until(&mut h, "the deleted thread to leave the panel", |cx| {
        restored.read_with(cx, |panel, _| panel.surfaces("t").is_none())
    });
    advance(&mut h, Duration::from_secs(1));
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(!text.contains(r#""t""#), "{text}");
}

#[gpui_kit::test]
fn the_resize_handle_moves_with_the_keyboard(cx: &mut TestAppContext) {
    let mut h = harness(cx, "resize-keys", 0);
    h.panel.update(&mut h.cx, |panel, cx| panel.toggle("t", cx));
    redraw(&mut h.cx);
    h.cx.update(|window, cx| window.focus_next(cx));
    redraw(&mut h.cx);
    let width = |h: &Harness| h.panel.read_with(&h.cx, |panel, _| panel.width());

    h.cx.simulate_keystrokes("left");
    redraw(&mut h.cx);
    assert_eq!(width(&h), DEFAULT_WIDTH + 16.);
    h.cx.simulate_keystrokes("right right");
    redraw(&mut h.cx);
    assert_eq!(width(&h), DEFAULT_WIDTH - 16.);
    h.cx.simulate_keystrokes("end");
    redraw(&mut h.cx);
    assert_eq!(width(&h), MAX_WIDTH);
    h.cx.simulate_keystrokes("home");
    redraw(&mut h.cx);
    assert_eq!(width(&h), super::MIN_WIDTH);
}

fn diff_files(h: &Harness) -> Entity<super::changes::ChangedFiles> {
    h.panel.read_with(&h.cx, |panel, cx| {
        panel
            .views
            .values()
            .find_map(|view| match view {
                SurfaceView::Diff { view, .. } => Some(view.read(cx).files().clone()),
                _ => None,
            })
            .expect("the diff view")
    })
}

fn open_notes(h: &mut Harness, rows_at_least: usize) {
    h.panel.update(&mut h.cx, |panel, cx| {
        panel.open_surface("t", SurfaceKind::Diff, cx)
    });
    wait_until(h, "the changed file", |cx| {
        cx.debug_bounds("changed-file-notes.txt").is_some()
    });
    click(h, "changed-file-notes.txt");
    wait_for_rows(h, rows_at_least);
}

fn select_notes(cx: &mut TestAppContext, name: &str, wrap: bool) -> String {
    let mut h = harness(cx, name, 0);
    h.cx.update(|_, cx| {
        hoshi_ui::theme::set_fonts(
            hoshi_ui::theme::Fonts {
                word_wrap: wrap,
                ..Default::default()
            },
            cx,
        )
    });
    open_notes(&mut h, 6);
    let header = h.cx.debug_bounds("changed-file-notes.txt").unwrap();
    let from = point(header.left() + px(120.), header.bottom() + px(4.));
    let to = point(header.right() - px(4.), header.bottom() + px(300.));
    h.cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    redraw(&mut h.cx);
    h.cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default());
    redraw(&mut h.cx);
    h.cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    redraw(&mut h.cx);
    h.cx.update(gpui_kit::base::TextSelection::selected_text)
}

#[gpui_kit::test]
fn diff_lines_are_selectable_without_their_numbers(cx: &mut TestAppContext) {
    let text = select_notes(cx, "select", true);
    assert!(
        text.ends_with("\none\ntwo\nthree\nfour"),
        "the lines in order, without numbers or signs: {text:?}"
    );
}

#[gpui_kit::test]
fn diff_lines_that_do_not_wrap_are_selectable_too(cx: &mut TestAppContext) {
    let text = select_notes(cx, "select-nowrap", false);
    assert!(
        text.ends_with("\none\ntwo\nthree\nfour"),
        "the lines in order: {text:?}"
    );
}

#[gpui_kit::test]
fn without_word_wrap_a_long_diff_scrolls_sideways(cx: &mut TestAppContext) {
    let mut h = harness(cx, "wide", 0);
    let long = "x".repeat(400);
    std::fs::write(
        h.dir.join("project").join("notes.txt"),
        format!("one\n{long}\n"),
    )
    .unwrap();
    h.cx.update(|_, cx| {
        hoshi_ui::theme::set_fonts(
            hoshi_ui::theme::Fonts {
                word_wrap: false,
                ..Default::default()
            },
            cx,
        )
    });
    open_notes(&mut h, 5);
    let thumb =
        h.cx.debug_bounds("diff-thumb-notes.txt")
            .expect("a sideways scrollbar under the file");
    let files = diff_files(&h);
    let scroll = |h: &Harness| files.read_with(&h.cx, |files, _| files.scroll_of("notes.txt"));
    assert_eq!(scroll(&h), px(0.));

    let header = h.cx.debug_bounds("changed-file-notes.txt").unwrap();
    h.cx.simulate_event(gpui_kit::ScrollEvent {
        position: point(header.left() + px(100.), header.bottom() + px(30.)),
        delta: gpui_kit::ScrollDelta::Pixels(point(px(-120.), px(0.))),
        ..Default::default()
    });
    redraw(&mut h.cx);
    assert_eq!(scroll(&h), px(120.));

    let start = thumb.center();
    h.cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    redraw(&mut h.cx);
    for x in [10., 4000.] {
        h.cx.simulate_mouse_move(
            point(start.x + px(x), start.y),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        redraw(&mut h.cx);
    }
    h.cx.simulate_mouse_up(
        point(start.x + px(4000.), start.y),
        MouseButton::Left,
        Modifiers::default(),
    );
    redraw(&mut h.cx);
    let end = scroll(&h);
    assert!(end > px(1000.), "dragged to {end:?}");
    let thumb = h.cx.debug_bounds("diff-thumb-notes.txt").unwrap();
    let panel = h.cx.debug_bounds("right-panel").unwrap();
    assert!(
        (thumb.right() - panel.right()).abs() < px(2.),
        "the thumb reaches the end"
    );
}

/// The forge as far as the panel's actions go: it records each change.
#[derive(Default)]
struct FakeForge {
    actions: std::sync::Mutex<Vec<hoshi_core::PullRequestAction>>,
    refuse: std::sync::Mutex<Option<String>>,
}

impl super::pull_request::PullRequestForge for FakeForge {
    fn act(
        &self,
        _: &str,
        forge_id: &str,
        action: &hoshi_core::PullRequestAction,
    ) -> Result<(), hoshi_core::CommandError> {
        assert_eq!(forge_id, "PR_1");
        if let Some(detail) = self.refuse.lock().unwrap().take() {
            return Err(hoshi_core::CommandError::ForgeRejected { detail });
        }
        self.actions.lock().unwrap().push(action.clone());
        Ok(())
    }

    fn reviewer_candidates(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Vec<String>, hoshi_core::CommandError> {
        Ok(["nxtvoid", "ana", "bo"].map(String::from).to_vec())
    }

    fn labels(
        &self,
        _: &str,
    ) -> Result<Vec<hoshi_core::PullRequestLabel>, hoshi_core::CommandError> {
        Ok(vec![
            hoshi_core::PullRequestLabel {
                name: "ui".into(),
                color: "0e8a16".into(),
            },
            hoshi_core::PullRequestLabel {
                name: "bug".into(),
                color: "d73a4a".into(),
            },
        ])
    }
}

fn pull_request_surface(
    h: &mut Harness,
    forge: Arc<FakeForge>,
) -> Entity<super::pull_request::PullRequestSurface> {
    h.workspace.update(&mut h.cx, |workspace, cx| {
        workspace.pull_requests.insert("t".into(), pull_request());
        workspace
            .pull_request_details
            .insert("t".into(), Some(detail()));
        cx.notify();
    });
    h.panel.update(&mut h.cx, |panel, cx| {
        panel.open_surface("t", SurfaceKind::PullRequest, cx)
    });
    redraw(&mut h.cx);
    let surface = h
        .panel
        .read_with(&h.cx, |panel, _| {
            panel.views.values().find_map(|view| match view {
                SurfaceView::PullRequest(view) => Some(view.clone()),
                _ => None,
            })
        })
        .expect("the pull request view");
    surface.update(&mut h.cx, |surface, _| surface.set_forge(forge));
    surface
}

fn actions(forge: &FakeForge) -> Vec<hoshi_core::PullRequestAction> {
    forge.actions.lock().unwrap().clone()
}

#[gpui_kit::test]
fn a_reply_goes_to_its_review_thread_and_a_refusal_shows_the_forges_words(cx: &mut TestAppContext) {
    use super::pull_request::Editing;
    use hoshi_core::PullRequestAction;
    let mut h = harness(cx, "pr-reply", 0);
    let forge = Arc::new(FakeForge::default());
    let surface = pull_request_surface(&mut h, forge.clone());
    let reply = Editing::Reply("r1".into());

    surface.update_in(&mut h.cx, |surface, window, cx| {
        surface.start_reply("r1".into(), window, cx);
        surface.type_into(&reply, "Renamed it", window, cx);
    });
    redraw(&mut h.cx);
    *forge.refuse.lock().unwrap() = Some("Thread is outdated".into());
    h.cx.simulate_keystrokes("ctrl-enter");
    redraw(&mut h.cx);
    assert_eq!(
        surface.read_with(&h.cx, |surface, _| surface.error().map(str::to_owned)),
        Some("Thread is outdated".into())
    );
    assert!(h.cx.debug_bounds("pr-error").is_some(), "the error shows");
    assert!(surface.read_with(&h.cx, |surface, _| surface.is_editing(&reply)));

    h.cx.simulate_keystrokes("ctrl-enter");
    redraw(&mut h.cx);
    assert_eq!(
        actions(&forge),
        [PullRequestAction::ReplyToThread {
            thread_id: "r1".into(),
            body: "Renamed it".into(),
        }]
    );
    assert!(h.cx.debug_bounds("pr-error").is_none(), "the error clears");
    assert!(!surface.read_with(&h.cx, |surface, _| surface.is_editing(&reply)));

    surface.update_in(&mut h.cx, |surface, window, cx| {
        surface.start_reply("r1".into(), window, cx)
    });
    redraw(&mut h.cx);
    h.cx.simulate_keystrokes("escape");
    redraw(&mut h.cx);
    assert!(!surface.read_with(&h.cx, |surface, _| surface.is_editing(&reply)));
}

#[gpui_kit::test]
fn the_title_saves_on_enter_and_comments_empty_once_sent(cx: &mut TestAppContext) {
    use super::pull_request::{Editing, Tab};
    use hoshi_core::PullRequestAction;
    let mut h = harness(cx, "pr-title", 0);
    let forge = Arc::new(FakeForge::default());
    let surface = pull_request_surface(&mut h, forge.clone());

    click(&mut h, "edit-title");
    assert!(surface.read_with(&h.cx, |surface, _| surface.is_editing(&Editing::Title)));
    surface.update_in(&mut h.cx, |surface, window, cx| {
        surface.type_into(&Editing::Title, "  Port the right panel ", window, cx)
    });
    h.cx.simulate_keystrokes("enter");
    redraw(&mut h.cx);
    assert_eq!(
        actions(&forge),
        [PullRequestAction::Edit {
            title: "Port the right panel".into(),
            body: "Adds **the panel**.".into(),
        }]
    );
    assert!(!surface.read_with(&h.cx, |surface, _| surface.is_editing(&Editing::Title)));

    surface.update(&mut h.cx, |surface, cx| surface.select(Tab::Timeline, cx));
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("pr-editor-comment").is_some());
    surface.update_in(&mut h.cx, |surface, window, cx| {
        surface.type_into(&Editing::Comment, "Looks good", window, cx);
        surface.save(Editing::Comment, window, cx);
    });
    redraw(&mut h.cx);
    assert_eq!(
        actions(&forge).last(),
        Some(&PullRequestAction::Comment {
            body: "Looks good".into()
        })
    );
    surface.update_in(&mut h.cx, |surface, window, cx| {
        surface.save(Editing::Comment, window, cx)
    });
    redraw(&mut h.cx);
    assert_eq!(actions(&forge).len(), 2, "the comment box was emptied");
}

#[gpui_kit::test]
fn chips_pickers_and_the_checkout_command(cx: &mut TestAppContext) {
    use hoshi_core::PullRequestAction;
    let mut h = harness(cx, "pr-pickers", 0);
    let forge = Arc::new(FakeForge::default());
    let mut detail = detail();
    detail.reviewers.push(hoshi_core::Reviewer {
        name: "cy".into(),
        state: hoshi_core::ReviewState::Requested,
    });
    let _surface = pull_request_surface(&mut h, forge.clone());
    h.workspace.update(&mut h.cx, |workspace, cx| {
        workspace
            .pull_request_details
            .insert("t".into(), Some(detail));
        cx.notify();
    });
    redraw(&mut h.cx);
    assert!(
        h.cx.debug_bounds("remove-reviewer-ana").is_none(),
        "who already reviewed stays"
    );
    click(&mut h, "remove-reviewer-cy");
    click(&mut h, "remove-label-ui");

    click(&mut h, "request-review");
    advance(&mut h, Duration::from_millis(300));
    assert!(
        h.cx.debug_bounds("reviewer-1").is_none(),
        "only bo is left to ask"
    );
    click(&mut h, "reviewer-0");
    assert!(
        h.cx.debug_bounds("reviewer-0").is_none(),
        "the picker closes"
    );

    click(&mut h, "edit-labels");
    click(&mut h, "label-1");
    assert_eq!(
        actions(&forge),
        [
            PullRequestAction::RemoveReviewers {
                names: vec!["cy".into()]
            },
            PullRequestAction::RemoveLabel { name: "ui".into() },
            PullRequestAction::RequestReviewers {
                names: vec!["bo".into()]
            },
            PullRequestAction::AddLabels {
                names: vec!["bug".into()]
            },
        ]
    );
    h.cx.simulate_keystrokes("escape");
    redraw(&mut h.cx);

    click(&mut h, "copy-checkout");
    let copied =
        h.cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(copied.as_deref(), Some("gh pr checkout 42"));
}

/// A key pressed and let go, as dialog buttons answer on release.
fn press(h: &mut Harness, key: &'static str) {
    let keystroke = gpui_kit::Keystroke::parse(key).unwrap();
    h.cx.simulate_event(gpui_kit::KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    h.cx.simulate_event(gpui_kit::KeyUpEvent { keystroke });
    redraw(&mut h.cx);
}

#[gpui_kit::test]
fn merging_asks_first_then_merges_with_the_chosen_method(cx: &mut TestAppContext) {
    use hoshi_core::{MergeMethod, PullRequestAction};
    let mut h = harness(cx, "pr-merge", 0);
    let forge = Arc::new(FakeForge::default());
    let _surface = pull_request_surface(&mut h, forge.clone());

    click(&mut h, "merge-menu");
    press(&mut h, "down");
    press(&mut h, "enter");
    advance(&mut h, Duration::from_millis(500));
    assert_eq!(
        h.cx.update(|window, cx| hoshi_ui::DialogLayer::open_count(window, cx)),
        1,
        "the merge asks first"
    );
    assert!(
        actions(&forge).is_empty(),
        "nothing merges before the answer"
    );
    press(&mut h, "enter");
    advance(&mut h, Duration::from_secs(1));
    assert_eq!(
        actions(&forge),
        [PullRequestAction::Merge {
            method: MergeMethod::Squash
        }]
    );
}
