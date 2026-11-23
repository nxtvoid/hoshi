use super::{PanelTerminals, TerminalDrawer, TerminalView, WorktreeSetupBar, store};
use crate::panel::{RightPanel, SurfaceKind};
use crate::state::{self, Workspace};
use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{
    AppContext as _, Bounds, Entity, EntityInputHandler as _, Modifiers, MouseButton, Pixels,
    Point, ScrollDelta, ScrollEvent, TestAppContext, TouchPhase, VisualTestContext, WindowBounds,
    WindowOptions, point, px, size,
};
use hoshi_core::{
    AccessMode, Command, CommandError, ThreadWorkspace, WorktreeSetupRun, WorktreeSetupStatus,
};
use hoshi_engine::Engine;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Harness {
    cx: VisualTestContext,
    drawer: Entity<TerminalDrawer>,
    workspace: Entity<Workspace>,
    engine: Arc<Engine>,
    home: PathBuf,
}

fn window_options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Point::default(),
            size: size(px(800.), px(400.)),
        })),
        ..Default::default()
    }
}

/// An engine on a scratch home with project `p` and thread `t` open.
fn workspace(cx: &mut TestAppContext, name: &str) -> (Entity<Workspace>, Arc<Engine>, PathBuf) {
    cx.executor().allow_parking();
    let dir = std::env::temp_dir().join(format!("hoshi-native-{name}-{}", uuid::Uuid::new_v4()));
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
    let workspace = {
        let engine = engine.clone();
        let settings_path = home.join("settings.json");
        cx.update(|cx| {
            cx.new(|cx| {
                let mut workspace = Workspace::new(engine, notifications, cx);
                workspace.settings_path = Some(settings_path);
                workspace
            })
        })
    };
    workspace.update(cx, |workspace, cx| workspace.open_thread("t", cx));
    (workspace, engine, home)
}

fn harness(cx: &mut TestAppContext, name: &str) -> Harness {
    let (workspace, engine, home) = workspace(cx, name);
    let (window, drawer) = {
        let workspace = workspace.clone();
        cx.update(|cx| {
            gpui_kit::open_window(window_options(), cx, |window, cx| {
                cx.new(|cx| TerminalDrawer::new(workspace, window, cx))
            })
            .unwrap()
        })
    };
    let cx = VisualTestContext::from_window(window, cx);
    Harness {
        cx,
        drawer,
        workspace,
        engine,
        home,
    }
}

fn active_id(h: &Harness) -> String {
    h.drawer
        .read_with(&h.cx, |drawer, _| drawer.drawer("t").active_id)
        .expect("a terminal shows")
}

fn view(h: &Harness, terminal_id: &str) -> Entity<TerminalView> {
    h.drawer
        .read_with(&h.cx, |drawer, _| drawer.attached(terminal_id))
        .expect("the terminal is attached")
}

fn screen(h: &Harness, terminal_id: &str) -> String {
    h.drawer.read_with(&h.cx, |drawer, cx| {
        drawer
            .attached(terminal_id)
            .map(|view| view.read(cx).screen_text().join("\n"))
            .unwrap_or_default()
    })
}

fn settle(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn wait_for(
    cx: &mut VisualTestContext,
    what: &str,
    mut done: impl FnMut(&mut VisualTestContext) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        settle(cx);
        if done(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn wait_until(h: &mut Harness, what: &str, mut done: impl FnMut(&mut Harness) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(40);
    loop {
        settle(&mut h.cx);
        if done(h) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn running(h: &Harness, terminal_id: &str) -> bool {
    h.drawer.read_with(&h.cx, |drawer, cx| {
        drawer
            .attached(terminal_id)
            .is_some_and(|view| view.read(cx).running())
    })
}

/// Opens the drawer and waits for its shell to start and go quiet.
fn started(h: &mut Harness) -> String {
    h.drawer
        .update(&mut h.cx, |drawer, cx| drawer.toggle("t", cx));
    let id = active_id(h);
    wait_until(h, "the shell to start", |h| running(h, &id));
    let mut last = String::new();
    let mut quiet = 0;
    wait_until(h, "the shell to go quiet", |h| {
        std::thread::sleep(Duration::from_millis(150));
        let now = screen(h, &id);
        quiet = if now == last && !now.trim().is_empty() {
            quiet + 1
        } else {
            0
        };
        last = now;
        quiet >= 4
    });
    id
}

/// The window position of the middle of a cell.
fn cell_center(h: &Harness, terminal_id: &str, line: usize, column: usize) -> Point<Pixels> {
    let (origin, width, height) = view(h, terminal_id)
        .read_with(&h.cx, |view, _| view.grid_origin())
        .expect("the grid is laid out");
    point(
        origin.x + width * (column as f32 + 0.5),
        origin.y + height * (line as f32 + 0.5),
    )
}

#[gpui_kit::test]
fn a_shell_runs_what_is_typed_and_shows_its_output(cx: &mut TestAppContext) {
    let mut h = harness(cx, "terminal-echo");
    h.drawer
        .update(&mut h.cx, |drawer, cx| drawer.toggle("t", cx));
    let id = active_id(&h);
    wait_until(&mut h, "the shell to start", |h| running(h, &id));

    let fitted = h.drawer.read_with(&h.cx, |drawer, cx| {
        drawer.attached(&id).unwrap().read(cx).grid_size()
    });
    assert!(fitted.columns > 80, "{fitted:?}");
    assert!(fitted.lines < 24, "{fitted:?}");

    std::thread::sleep(Duration::from_millis(500));
    h.cx.simulate_input("echo hoshi$((6*7))");
    h.cx.simulate_keystrokes("enter");
    wait_until(&mut h, "the echo", |h| {
        screen(h, &id).lines().any(|line| line.trim() == "hoshi42")
    });
}

#[gpui_kit::test]
fn hiding_the_drawer_detaches_and_reopening_redraws_the_history(cx: &mut TestAppContext) {
    let mut h = harness(cx, "terminal-reattach");
    h.drawer
        .update(&mut h.cx, |drawer, cx| drawer.toggle("t", cx));
    let id = active_id(&h);
    wait_until(&mut h, "the shell to start", |h| running(h, &id));
    std::thread::sleep(Duration::from_millis(500));
    h.cx.simulate_input("echo keptacrossviews");
    h.cx.simulate_keystrokes("enter");
    wait_until(&mut h, "the echo", |h| {
        screen(h, &id)
            .lines()
            .any(|line| line.trim() == "keptacrossviews")
    });

    h.drawer
        .update(&mut h.cx, |drawer, cx| drawer.toggle("t", cx));
    settle(&mut h.cx);
    let attached = h
        .drawer
        .read_with(&h.cx, |drawer, _| drawer.attached(&id).is_some());
    assert!(!attached, "a hidden drawer keeps no view");
    assert!(
        h.engine.write_terminal(&id, "").is_ok(),
        "the shell keeps running"
    );

    h.drawer
        .update(&mut h.cx, |drawer, cx| drawer.toggle("t", cx));
    wait_until(&mut h, "the history", |h| {
        screen(h, &id)
            .lines()
            .any(|line| line.trim() == "keptacrossviews")
    });
}

#[gpui_kit::test]
fn tabs_add_switch_and_close_their_shells(cx: &mut TestAppContext) {
    let mut h = harness(cx, "terminal-tabs");
    h.drawer
        .update(&mut h.cx, |drawer, cx| drawer.toggle("t", cx));
    let first = active_id(&h);
    wait_until(&mut h, "the first shell", |h| running(h, &first));
    h.drawer
        .update(&mut h.cx, |drawer, cx| drawer.add_terminal("t", cx));
    let second = active_id(&h);
    assert_ne!(first, second);
    wait_until(&mut h, "the second shell", |h| running(h, &second));

    h.drawer.update(&mut h.cx, |drawer, cx| {
        drawer.close_terminal("t", &second, cx)
    });
    h.cx.run_until_parked();
    let drawer = h.drawer.read_with(&h.cx, |drawer, _| drawer.drawer("t"));
    assert_eq!(drawer.terminal_ids, vec![first.clone()]);
    assert_eq!(drawer.active_id.as_deref(), Some(first.as_str()));
    assert!(matches!(
        h.engine.write_terminal(&second, "x"),
        Err(CommandError::TerminalNotFound { .. })
    ));

    h.drawer.update(&mut h.cx, |drawer, cx| {
        drawer.close_terminal("t", &first, cx)
    });
    h.cx.run_until_parked();
    assert!(!h.drawer.read_with(&h.cx, |drawer, _| drawer.is_open("t")));
}

#[gpui_kit::test]
fn a_terminal_the_engine_started_opens_in_its_own_tab(cx: &mut TestAppContext) {
    let mut h = harness(cx, "terminal-show");
    h.drawer.update(&mut h.cx, |drawer, cx| {
        drawer.show_terminal("t", "setup", cx)
    });
    h.cx.run_until_parked();
    let drawer = h.drawer.read_with(&h.cx, |drawer, _| drawer.drawer("t"));
    assert!(drawer.open);
    assert_eq!(drawer.terminal_ids, vec!["setup".to_owned()]);
    assert_eq!(drawer.active_id.as_deref(), Some("setup"));
    h.engine.close_terminal("setup");
}

#[gpui_kit::test]
fn the_setup_bar_opens_the_worktree_setup_terminal_in_the_drawer(cx: &mut TestAppContext) {
    let mut h = harness(cx, "terminal-setup-bar");
    let run = |status| WorktreeSetupRun {
        thread_id: "t".into(),
        terminal_id: Some("setup".into()),
        status,
        exit_code: None,
        copied_files: 2,
        missing_files: Vec::new(),
        failed_files: Vec::new(),
        output: vec!["installing".into()],
        held_messages: 1,
    };
    h.workspace.update(&mut h.cx, |workspace, cx| {
        workspace
            .worktree_setups
            .insert("t".into(), run(WorktreeSetupStatus::Failed));
        cx.notify();
    });
    let bar = {
        let (workspace, drawer) = (h.workspace.clone(), h.drawer.clone());
        h.cx.update(|_, cx| cx.new(|cx| WorktreeSetupBar::new(workspace, &drawer, cx)))
    };
    bar.update(&mut h.cx, |bar, cx| bar.click_open_terminal(cx));
    h.cx.run_until_parked();
    let drawer = h.drawer.read_with(&h.cx, |drawer, _| drawer.drawer("t"));
    assert!(drawer.open);
    assert_eq!(drawer.active_id.as_deref(), Some("setup"));

    bar.update(&mut h.cx, |bar, cx| bar.click_dismiss(cx));
    assert!(bar.read_with(&h.cx, |bar, cx| bar.shown(cx).is_none()));
    h.workspace.update(&mut h.cx, |workspace, cx| {
        workspace
            .worktree_setups
            .insert("t".into(), run(WorktreeSetupStatus::Running));
        cx.notify();
    });
    h.cx.run_until_parked();
    assert!(
        bar.read_with(&h.cx, |bar, cx| bar.shown(cx).is_some()),
        "a setup that runs again shows again"
    );
    h.engine.close_terminal("setup");
}

#[gpui_kit::test]
fn drawers_and_height_survive_a_restart_until_their_thread_is_deleted(cx: &mut TestAppContext) {
    let mut h = harness(cx, "terminal-persist");
    let id = started(&mut h);
    h.drawer
        .update(&mut h.cx, |drawer, cx| drawer.set_height(333., cx));
    h.cx.executor().advance_clock(Duration::from_millis(400));
    h.cx.run_until_parked();
    let saved = store::load(&h.home);
    assert_eq!(saved.height, 333.);
    assert_eq!(saved.drawers["t"].terminal_ids, vec![id.clone()]);
    assert!(saved.drawers["t"].open);

    let restarted = {
        let workspace = h.workspace.clone();
        h.cx.update(|window, cx| cx.new(|cx| TerminalDrawer::new(workspace, window, cx)))
    };
    restarted.read_with(&h.cx, |drawer, _| {
        assert_eq!(drawer.height, 333.);
        assert_eq!(drawer.drawer("t").active_id, Some(id.clone()));
    });

    h.engine
        .dispatch(Command::ThreadDelete {
            thread_id: "t".into(),
        })
        .unwrap();
    wait_for(&mut h.cx, "the deleted thread's drawer to go", |cx| {
        restarted.read_with(cx, |drawer, _| drawer.drawers.is_empty())
    });
    h.cx.executor().advance_clock(Duration::from_millis(400));
    h.cx.run_until_parked();
    assert!(store::load(&h.home).drawers.is_empty());
}

#[gpui_kit::test]
fn a_program_that_reads_the_mouse_gets_clicks_and_the_wheel(cx: &mut TestAppContext) {
    let mut h = harness(cx, "terminal-mouse");
    let id = started(&mut h);
    let view = view(&h, &id);
    view.update(&mut h.cx, |view, cx| {
        view.feed("\x1b[?1000h\x1b[?1006h", cx);
        view.take_sent();
    });
    settle(&mut h.cx);
    let at = cell_center(&h, &id, 1, 2);
    h.cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    h.cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    h.cx.simulate_event(ScrollEvent {
        position: at,
        delta: ScrollDelta::Lines(point(0., 2.)),
        modifiers: Modifiers::default(),
        touch_phase: TouchPhase::Moved,
        ..Default::default()
    });
    let reports: Vec<String> = view
        .read_with(&h.cx, |view, _| view.take_sent())
        .into_iter()
        .filter(|sent| sent.starts_with("\x1b[<"))
        .collect();
    assert_eq!(
        reports,
        vec![
            "\x1b[<0;3;2M",
            "\x1b[<0;3;2m",
            "\x1b[<64;3;2M",
            "\x1b[<64;3;2M"
        ]
    );

    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    h.cx.simulate_mouse_down(at, MouseButton::Left, shift);
    h.cx.simulate_mouse_up(at, MouseButton::Left, shift);
    let sent = view.read_with(&h.cx, |view, _| view.take_sent());
    assert!(
        !sent.iter().any(|sent| sent.starts_with("\x1b[<")),
        "Shift keeps the mouse for selecting: {sent:?}"
    );
}

#[gpui_kit::test]
fn a_link_underlines_under_the_pointer_and_ctrl_click_opens_it(cx: &mut TestAppContext) {
    let mut h = harness(cx, "terminal-links");
    let id = started(&mut h);
    let view = view(&h, &id);
    view.update(&mut h.cx, |view, cx| {
        view.feed("\r\nread https://example.com/docs. now\r\n", cx)
    });
    settle(&mut h.cx);
    let text = screen(&h, &id);
    let (line, column) = text
        .lines()
        .enumerate()
        .find_map(|(line, row)| Some((line, row.find("https://")?)))
        .unwrap_or_else(|| panic!("the address is on screen: {text}"));
    let at = cell_center(&h, &id, line, column + 4);
    h.cx.simulate_mouse_move(at, None, Modifiers::default());
    assert_eq!(
        view.read_with(&h.cx, |view, _| view.hovered_link().map(str::to_owned)),
        Some("https://example.com/docs".to_owned())
    );
    let away = cell_center(&h, &id, line, column.saturating_sub(2));
    h.cx.simulate_mouse_move(away, None, Modifiers::default());
    assert!(view.read_with(&h.cx, |view, _| view.hovered_link().is_none()));

    h.cx.simulate_mouse_move(at, None, Modifiers::default());
    h.cx.simulate_click(at, Modifiers::control());
    settle(&mut h.cx);
    assert_eq!(
        h.cx.opened_url().as_deref(),
        Some("https://example.com/docs")
    );
}

#[gpui_kit::test]
fn the_input_method_composes_at_the_cursor_until_it_commits(cx: &mut TestAppContext) {
    let mut h = harness(cx, "terminal-ime");
    let id = started(&mut h);
    let view = view(&h, &id);
    view.update(&mut h.cx, |view, _| view.take_sent());
    h.cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.replace_and_mark_text_in_range(None, "にほ", None, window, cx)
        })
    });
    settle(&mut h.cx);
    assert_eq!(
        view.read_with(&h.cx, |view, _| view.preedit().map(str::to_owned)),
        Some("にほ".to_owned())
    );
    assert!(view.read_with(&h.cx, |view, _| view.take_sent()).is_empty());
    h.cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.replace_text_in_range(None, "日本", window, cx)
        })
    });
    settle(&mut h.cx);
    assert!(view.read_with(&h.cx, |view, _| view.preedit().is_none()));
    assert_eq!(
        view.read_with(&h.cx, |view, _| view.take_sent()),
        vec!["日本".to_owned()]
    );
}

#[gpui_kit::test]
fn the_focused_cursor_blinks_and_typing_shows_it(cx: &mut TestAppContext) {
    let mut h = harness(cx, "terminal-blink");
    let id = started(&mut h);
    let view = view(&h, &id);
    let shown = |h: &Harness| view.read_with(&h.cx, |view, _| view.cursor_shown());
    assert!(shown(&h));
    h.cx.executor().advance_clock(Duration::from_millis(650));
    h.cx.run_until_parked();
    assert!(!shown(&h), "it blinks off");
    h.cx.executor().advance_clock(Duration::from_millis(600));
    h.cx.run_until_parked();
    assert!(shown(&h), "and on again");
    h.cx.executor().advance_clock(Duration::from_millis(600));
    h.cx.run_until_parked();
    assert!(!shown(&h));
    h.cx.simulate_input("x");
    assert!(shown(&h), "typing shows it");
}

#[gpui_kit::test]
fn with_reduced_motion_the_cursor_stays_shown(cx: &mut TestAppContext) {
    let mut h = harness(cx, "terminal-still");
    let id = started(&mut h);
    let view = view(&h, &id);
    h.cx.update(|_, cx| cx.set_reduce_motion(true));
    h.cx.simulate_input("x");
    for _ in 0..3 {
        h.cx.executor().advance_clock(Duration::from_millis(650));
        h.cx.run_until_parked();
        assert!(view.read_with(&h.cx, |view, _| view.cursor_shown()));
    }
}

#[gpui_kit::test]
fn an_update_the_program_never_ends_still_draws(cx: &mut TestAppContext) {
    let mut h = harness(cx, "terminal-sync");
    let id = started(&mut h);
    let view = view(&h, &id);
    view.update(&mut h.cx, |view, cx| {
        view.feed("\x1b[?2026h\r\nsyncedtext", cx)
    });
    settle(&mut h.cx);
    assert!(!screen(&h, &id).contains("syncedtext"));
    std::thread::sleep(Duration::from_millis(200));
    h.cx.executor().advance_clock(Duration::from_millis(200));
    settle(&mut h.cx);
    assert!(screen(&h, &id).contains("syncedtext"));
}

#[gpui_kit::test]
fn the_panel_opens_runs_and_closes_terminals_through_its_host(cx: &mut TestAppContext) {
    let (workspace, engine, _) = workspace(cx, "terminal-panel");
    let (window, panel) = {
        let workspace = workspace.clone();
        cx.update(|cx| {
            gpui_kit::open_window(window_options(), cx, |window, cx| {
                cx.new(|cx| RightPanel::new(workspace, window, cx))
            })
            .unwrap()
        })
    };
    let host = cx.update(|cx| PanelTerminals::register(&workspace, &panel, cx));
    let mut cx = VisualTestContext::from_window(window, cx);
    panel.update(&mut cx, |panel, cx| {
        panel.open_surface("t", SurfaceKind::Terminal, cx)
    });
    let id = panel
        .read_with(&cx, |panel, _| {
            panel
                .surfaces("t")
                .and_then(|surfaces| surfaces.active_id.clone())
        })
        .expect("a terminal surface");
    wait_for(&mut cx, "the panel's shell", |cx| {
        host.view(&id)
            .is_some_and(|view| view.read_with(cx, |view, _| view.running()))
    });
    std::thread::sleep(Duration::from_millis(800));
    cx.simulate_input("echo panel$((40+2))");
    cx.simulate_keystrokes("enter");
    wait_for(&mut cx, "the echo in the panel", |cx| {
        host.view(&id).is_some_and(|view| {
            view.read_with(cx, |view, _| {
                view.screen_text()
                    .iter()
                    .any(|line| line.trim() == "panel42")
            })
        })
    });

    panel.update(&mut cx, |panel, cx| panel.close("t", cx));
    settle(&mut cx);
    assert!(host.view(&id).is_none(), "a closed panel keeps no view");
    assert!(
        engine.write_terminal(&id, "").is_ok(),
        "its shell keeps running"
    );

    panel.update(&mut cx, |panel, cx| panel.close_surface("t", &id, cx));
    settle(&mut cx);
    assert!(matches!(
        engine.write_terminal(&id, "x"),
        Err(CommandError::TerminalNotFound { .. })
    ));
}

#[gpui_kit::test]
fn a_drafts_terminal_starts_in_its_project_and_stays_with_the_thread_it_creates(
    cx: &mut TestAppContext,
) {
    let mut h = harness(cx, "terminal-draft");
    let draft_id = h.workspace.update(&mut h.cx, |workspace, cx| {
        workspace.start_draft(Some("p".into()), cx);
        workspace
            .shown_thread_id(cx)
            .expect("the draft has a thread id")
    });
    h.drawer
        .update(&mut h.cx, |drawer, cx| drawer.toggle(&draft_id, cx));
    let id = h
        .drawer
        .read_with(&h.cx, |drawer, _| drawer.drawer(&draft_id).active_id)
        .expect("a terminal shows");
    wait_until(&mut h, "the shell to start", |h| running(h, &id));
    let started_in = view(&h, &id).read_with(&h.cx, |view, _| view.started_in().map(PathBuf::from));
    assert_eq!(started_in, Some(h.home.parent().unwrap().join("project")));

    let again = h.workspace.update(&mut h.cx, |workspace, cx| {
        workspace.open_thread("t", cx);
        workspace.start_draft(Some("p".into()), cx);
        workspace.shown_thread_id(cx)
    });
    assert_eq!(again.as_deref(), Some(draft_id.as_str()));
    settle(&mut h.cx);
    assert!(h.engine.write_terminal(&id, "").is_ok());

    h.engine
        .dispatch(Command::ThreadCreate {
            thread_id: draft_id.clone(),
            project_id: Some("p".into()),
            title: String::new(),
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
    wait_until(&mut h, "the thread to be created", |h| {
        h.workspace.read_with(&h.cx, |workspace, _| {
            workspace.model.thread(&draft_id).is_some()
        })
    });
    h.workspace.update(&mut h.cx, |workspace, cx| {
        workspace.open_thread(&draft_id, cx)
    });
    settle(&mut h.cx);
    let drawer = h
        .drawer
        .read_with(&h.cx, |drawer, _| drawer.drawer(&draft_id));
    assert_eq!(drawer.terminal_ids, vec![id.clone()]);
    assert!(h.engine.write_terminal(&id, "").is_ok());
}

#[gpui_kit::test]
fn discarding_a_draft_ends_its_terminals(cx: &mut TestAppContext) {
    let mut h = harness(cx, "terminal-draft-discard");
    let draft_id = h.workspace.update(&mut h.cx, |workspace, cx| {
        workspace.start_draft(Some("p".into()), cx);
        workspace
            .shown_thread_id(cx)
            .expect("the draft has a thread id")
    });
    h.drawer
        .update(&mut h.cx, |drawer, cx| drawer.toggle(&draft_id, cx));
    let id = h
        .drawer
        .read_with(&h.cx, |drawer, _| drawer.drawer(&draft_id).active_id)
        .expect("a terminal shows");
    wait_until(&mut h, "the shell to start", |h| running(h, &id));

    let fresh = h.workspace.update(&mut h.cx, |workspace, cx| {
        workspace.forget_drafts(&[crate::drafts::key(None, Some("p"))], cx);
        workspace.shown_thread_id(cx)
    });
    settle(&mut h.cx);
    assert!(fresh.is_some_and(|fresh| fresh != draft_id));
    let drawer = h
        .drawer
        .read_with(&h.cx, |drawer, _| drawer.drawer(&draft_id));
    assert!(drawer.terminal_ids.is_empty());
    wait_until(&mut h, "the shell to end", |h| {
        h.engine.write_terminal(&id, "").is_err()
    });
}

#[gpui_kit::test]
fn removing_the_project_ends_its_drafts_terminals(cx: &mut TestAppContext) {
    let mut h = harness(cx, "terminal-draft-project-removed");
    let draft_id = h.workspace.update(&mut h.cx, |workspace, cx| {
        workspace.start_draft(Some("p".into()), cx);
        workspace
            .shown_thread_id(cx)
            .expect("the draft has a thread id")
    });
    h.drawer
        .update(&mut h.cx, |drawer, cx| drawer.toggle(&draft_id, cx));
    let id = h
        .drawer
        .read_with(&h.cx, |drawer, _| drawer.drawer(&draft_id).active_id)
        .expect("a terminal shows");
    wait_until(&mut h, "the shell to start", |h| running(h, &id));

    h.engine
        .dispatch(Command::ProjectRemove {
            project_id: "p".into(),
        })
        .unwrap();
    wait_until(&mut h, "the shell to end", |h| {
        h.engine.write_terminal(&id, "").is_err()
    });
    let drawer = h
        .drawer
        .read_with(&h.cx, |drawer, _| drawer.drawer(&draft_id));
    assert!(drawer.terminal_ids.is_empty());
}
