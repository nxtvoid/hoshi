use super::{Mode, Palette, PaletteEvent};
use crate::state::{self, Workspace};
use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement as _, Point, Render, Styled as _, Subscription, TestAppContext,
    VisualTestContext, Window, WindowBounds, WindowOptions, div, px, size,
};
use hoshi_core::{AccessMode, Command, ThreadWorkspace};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Shell {
    workspace: Entity<Workspace>,
    palette: Option<(Entity<Palette>, Subscription)>,
    settings_opened: bool,
    focus: FocusHandle,
}

impl Shell {
    fn open(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = self.workspace.clone();
        let palette = cx.new(|cx| Palette::new(workspace, mode, window, cx));
        let events = cx.subscribe(&palette, |this, _, event, cx| {
            match event {
                PaletteEvent::Close => this.palette = None,
                PaletteEvent::OpenSettings(_) | PaletteEvent::OpenUsage => {
                    this.palette = None;
                    this.settings_opened = true;
                }
                PaletteEvent::ShowView { .. } => {}
            }
            cx.notify();
        });
        self.palette = Some((palette, events));
        cx.notify();
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus)
            .size_full()
            .children(self.palette.as_ref().map(|(palette, _)| palette.clone()))
    }
}

struct Harness {
    cx: VisualTestContext,
    shell: Entity<Shell>,
    workspace: Entity<Workspace>,
    dir: PathBuf,
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

fn harness(cx: &mut TestAppContext, name: &str) -> Harness {
    cx.executor().allow_parking();
    let dir = std::env::temp_dir().join(format!("hoshi-native-{name}-{}", uuid::Uuid::new_v4()));
    let home = dir.join("home");
    for folder in ["project", "alpha", "alpine", "beta"] {
        std::fs::create_dir_all(dir.join(folder)).unwrap();
    }
    std::fs::create_dir_all(&home).unwrap();
    let (notify, notifications) = state::channel();
    let engine = Arc::new(hoshi_engine::Engine::open(&home, notify).unwrap());
    engine
        .dispatch(Command::ProjectCreate {
            project_id: "p".into(),
            title: "project".into(),
            workspace_root: dir.join("project").to_string_lossy().into_owned(),
        })
        .unwrap();
    engine.dispatch(thread("a", "first thread")).unwrap();
    engine.dispatch(thread("b", "second thread")).unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        hoshi_ui::DialogLayer::init(cx);
        super::init(cx);
        hoshi_ui::theme::apply(ThemeMode::Light, cx);
    });
    let projects_folder = dir.to_string_lossy().into_owned();
    let workspace = cx.update(|cx| {
        cx.new(|cx| {
            let mut workspace = Workspace::new(engine, notifications, cx);
            workspace.settings.general.projects_folder = Some(projects_folder);
            workspace
        })
    });
    let (window, shell) = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(900.), px(700.)),
            })),
            ..Default::default()
        };
        let mut shell = None;
        let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            let shown = cx.new(|_| Shell {
                workspace: workspace.clone(),
                palette: None,
                settings_opened: false,
                focus,
            });
            shell = Some(shown.clone());
            shown
        })
        .unwrap();
        (window, shell.unwrap())
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    redraw(&mut cx);
    Harness {
        cx,
        shell,
        workspace,
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

fn open(h: &mut Harness, mode: Mode) {
    let shell = h.shell.clone();
    h.cx.update(|window, cx| shell.update(cx, |shell, cx| shell.open(mode, window, cx)));
    redraw(&mut h.cx);
}

fn palette(h: &Harness) -> Option<Entity<Palette>> {
    h.shell.read_with(&h.cx, |shell, _| {
        shell.palette.as_ref().map(|(palette, _)| palette.clone())
    })
}

fn query(h: &Harness) -> String {
    palette(h)
        .expect("the palette is open")
        .read_with(&h.cx, |palette, _| palette.query.clone())
}

fn mode(h: &Harness) -> Mode {
    palette(h)
        .expect("the palette is open")
        .read_with(&h.cx, |palette, _| palette.mode)
}

fn drawn(h: &mut Harness, key: &str) -> bool {
    let selector: &'static str = Box::leak(format!("palette-{key}").into_boxed_str());
    h.cx.debug_bounds(selector).is_some()
}

fn type_text(h: &mut Harness, text: &str) {
    h.cx.simulate_input(text);
    redraw(&mut h.cx);
}

#[gpui_kit::test]
fn typing_narrows_the_threads_and_enter_opens_one(cx: &mut TestAppContext) {
    let mut h = harness(cx, "palette-search");
    open(&mut h, Mode::Commands);
    assert!(drawn(&mut h, "thread:a"));
    assert!(drawn(&mut h, "action:AddProject"));
    type_text(&mut h, "second");
    assert!(drawn(&mut h, "thread:b"));
    assert!(!drawn(&mut h, "thread:a"));
    h.cx.simulate_keystrokes("enter");
    redraw(&mut h.cx);
    assert!(palette(&h).is_none(), "the palette closes");
    let open = h
        .workspace
        .read_with(&h.cx, |workspace, cx| workspace.open_thread_id(cx));
    assert_eq!(open.as_deref(), Some("b"));
}

#[gpui_kit::test]
fn the_arrows_move_the_highlight_and_escape_closes(cx: &mut TestAppContext) {
    let mut h = harness(cx, "palette-arrows");
    open(&mut h, Mode::Commands);
    let active = |h: &Harness| {
        palette(h)
            .unwrap()
            .read_with(&h.cx, |palette, _| palette.active)
    };
    assert_eq!(active(&h), Some(0));
    h.cx.simulate_keystrokes("down down");
    assert_eq!(active(&h), Some(2));
    h.cx.simulate_keystrokes("up up up");
    assert!(active(&h).unwrap() > 2, "up from the top wraps to the end");
    h.cx.simulate_keystrokes("escape");
    redraw(&mut h.cx);
    assert!(palette(&h).is_none());
}

#[gpui_kit::test]
fn backspace_on_an_empty_field_goes_back(cx: &mut TestAppContext) {
    let mut h = harness(cx, "palette-back");
    open(&mut h, Mode::Commands);
    type_text(&mut h, "new thread");
    h.cx.simulate_keystrokes("down");
    let palette_entity = palette(&h).unwrap();
    h.cx.update(|window, cx| {
        palette_entity.update(cx, |palette, cx| {
            palette.set_mode(Mode::NewThread, window, cx)
        })
    });
    redraw(&mut h.cx);
    assert!(drawn(&mut h, "chat"));
    assert!(drawn(&mut h, "p"));
    h.cx.simulate_keystrokes("backspace");
    redraw(&mut h.cx);
    assert_eq!(mode(&h), Mode::Commands);
}

#[gpui_kit::test]
fn a_new_thread_starts_where_the_palette_points(cx: &mut TestAppContext) {
    let mut h = harness(cx, "palette-new-thread");
    open(&mut h, Mode::NewThread);
    assert!(drawn(&mut h, "chat"));
    h.cx.simulate_keystrokes("down");
    h.cx.simulate_keystrokes("enter");
    redraw(&mut h.cx);
    assert!(palette(&h).is_none());
    let draft = h
        .workspace
        .read_with(&h.cx, |workspace, _| workspace.draft.clone());
    assert_eq!(
        draft.and_then(|draft| draft.project_id).as_deref(),
        Some("p")
    );
}

#[gpui_kit::test]
fn tab_completes_a_folder_and_enter_adds_a_new_one(cx: &mut TestAppContext) {
    let mut h = harness(cx, "palette-folder");
    open(&mut h, Mode::AddProject);
    assert!(drawn(&mut h, "new"));
    type_text(&mut h, "local");
    h.cx.simulate_keystrokes("enter");
    let base = format!(
        "{}{}",
        h.dir.to_string_lossy(),
        super::add_project::SEPARATOR
    );
    wait_for(&mut h, "the projects folder", |h| query(h) == base);
    wait_for(&mut h, "its folders", |h| drawn(h, "folder:alpha"));
    assert!(drawn(&mut h, "up"));
    type_text(&mut h, "al");
    assert!(drawn(&mut h, "folder:alpine"));
    assert!(!drawn(&mut h, "folder:beta"));
    h.cx.simulate_keystrokes("tab");
    redraw(&mut h.cx);
    assert_eq!(query(&h), format!("{base}alp"));
    type_text(&mut h, "h");
    h.cx.simulate_keystrokes("tab");
    let alpha = format!("{base}alpha{}", super::add_project::SEPARATOR);
    wait_for(&mut h, "alpha to open", |h| query(h) == alpha);
    type_text(&mut h, "fresh");
    h.cx.simulate_keystrokes("enter");
    let created = h.dir.join("alpha").join("fresh");
    wait_for(&mut h, "the project to be added", |h| {
        h.workspace.read_with(&h.cx, |workspace, _| {
            workspace
                .model
                .projects()
                .any(|project| std::path::Path::new(&project.workspace_root) == created)
        })
    });
    assert!(created.is_dir());
    assert!(palette(&h).is_none());
}

#[gpui_kit::test]
fn a_bad_git_url_says_why_and_a_good_one_asks_where(cx: &mut TestAppContext) {
    let mut h = harness(cx, "palette-clone");
    open(&mut h, Mode::AddProject);
    type_text(&mut h, "git url");
    h.cx.simulate_keystrokes("enter");
    redraw(&mut h.cx);
    type_text(&mut h, "nothing");
    h.cx.simulate_keystrokes("enter");
    redraw(&mut h.cx);
    let error = palette(&h)
        .unwrap()
        .read_with(&h.cx, |palette, _| palette.add.error.clone());
    assert_eq!(error, Some(crate::m::error_git_url_invalid()));
    h.cx.simulate_keystrokes("ctrl-a backspace");
    type_text(&mut h, "https://example.com/nxtvoid/demo.git");
    h.cx.simulate_keystrokes("enter");
    let expected = h.dir.join("demo").to_string_lossy().into_owned();
    wait_for(&mut h, "the destination step", |h| query(h) == expected);
    h.cx.simulate_keystrokes("backspace");
    redraw(&mut h.cx);
    assert_eq!(query(&h), expected[..expected.len() - 1]);
}

#[gpui_kit::test]
fn a_signed_out_forge_offers_its_settings(cx: &mut TestAppContext) {
    let mut h = harness(cx, "palette-forge");
    open(&mut h, Mode::AddProject);
    assert!(drawn(&mut h, "github"));
    let setup =
        h.cx.debug_bounds("palette-setup-required")
            .expect("a setup button")
            .center();
    h.cx.simulate_click(setup, gpui_kit::Modifiers::none());
    redraw(&mut h.cx);
    assert!(h.shell.read_with(&h.cx, |shell, _| shell.settings_opened));
}
