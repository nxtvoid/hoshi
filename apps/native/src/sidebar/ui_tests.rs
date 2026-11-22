use super::{CardTarget, Sidebar, prefs, shortcuts};
use crate::sections::SectionId;
use crate::state::{self, Workspace};
use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, InteractiveElement as _, IntoElement, Modifiers,
    MouseButton, ParentElement as _, Point, Render, Styled as _, TestAppContext, VisualTestContext,
    Window, WindowBounds, WindowOptions, div, px, size,
};
use hoshi_core::{AccessMode, Command, ThreadWorkspace};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Shell {
    sidebar: Entity<Sidebar>,
    toasts: Entity<crate::toast::Toasts>,
    focus: FocusHandle,
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        shortcuts(div(), &self.sidebar)
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .child(self.sidebar.clone())
            .child(
                div()
                    .id("main")
                    .debug_selector(|| "main".into())
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .child(self.toasts.clone()),
            )
    }
}

struct Harness {
    cx: VisualTestContext,
    sidebar: Entity<Sidebar>,
    workspace: Entity<Workspace>,
    home: PathBuf,
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
    std::fs::create_dir_all(dir.join("project")).unwrap();
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
    engine.dispatch(thread("a", "first")).unwrap();
    engine.dispatch(thread("b", "second")).unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        hoshi_ui::DialogLayer::init(cx);
        super::init(cx);
        hoshi_ui::theme::apply(ThemeMode::Light, cx);
    });
    let settings_path = home.join("settings.json");
    let workspace = cx.update(|cx| {
        cx.new(|cx| {
            let mut workspace = Workspace::new(engine, notifications, cx);
            workspace.settings_path = Some(settings_path);
            workspace
        })
    });
    let (window, sidebar) = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(900.), px(600.)),
            })),
            ..Default::default()
        };
        let mut sidebar = None;
        let toasts = crate::toast::install(cx);
        let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
            let shown = cx.new(|cx| Sidebar::new(workspace.clone(), window, cx));
            sidebar = Some(shown.clone());
            let focus = cx.focus_handle();
            window.focus(&focus, cx);
            cx.new(|_| Shell {
                sidebar: shown,
                toasts,
                focus,
            })
        })
        .unwrap();
        (window, sidebar.unwrap())
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    redraw(&mut cx);
    Harness {
        cx,
        sidebar,
        workspace,
        home,
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

fn center(h: &mut Harness, selector: &'static str) -> Point<Pixels> {
    h.cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
        .center()
}

use gpui_kit::{FocusHandle, Pixels};

#[gpui_kit::test]
fn without_projects_only_the_project_view_asks_for_one(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-no-projects");
    let engine = h
        .workspace
        .read_with(&h.cx, |workspace, _| workspace.engine.clone());
    engine
        .dispatch(Command::ProjectRemove {
            project_id: "p".into(),
        })
        .unwrap();
    wait_for(&mut h, "the project to go", |h| {
        h.workspace.read_with(&h.cx, |workspace, _| {
            workspace.model.projects().next().is_none()
        })
    });
    assert!(h.cx.debug_bounds("no-projects").is_some());

    for view in [crate::sections::View::Inbox, crate::sections::View::Chats] {
        h.sidebar
            .update(&mut h.cx, |sidebar, cx| sidebar.set_view(view, cx));
        redraw(&mut h.cx);
        assert!(h.cx.debug_bounds("no-projects").is_none(), "{view:?}");
    }
}

#[gpui_kit::test]
fn a_right_click_opens_the_thread_menu_and_pins_it(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-pin");
    let row = center(&mut h, "thread-a");
    h.cx.simulate_mouse_down(row, MouseButton::Right, Modifiers::none());
    h.cx.simulate_mouse_up(row, MouseButton::Right, Modifiers::none());
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("menu-item-thread-delete").is_some());
    let pin = center(&mut h, "menu-item-thread-pin");
    h.cx.simulate_click(pin, Modifiers::none());
    wait_for(&mut h, "the thread to be pinned", |h| {
        h.workspace.read_with(&h.cx, |workspace, _| {
            workspace.model.thread("a").unwrap().pinned
        })
    });
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("menu-item-thread-pin").is_none());
    assert!(h.cx.debug_bounds("section-Pinned").is_some());
}

#[gpui_kit::test]
fn a_right_click_on_the_rail_opens_the_project_menu(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-project");
    let button = center(&mut h, "rail-p");
    h.cx.simulate_mouse_down(button, MouseButton::Right, Modifiers::none());
    h.cx.simulate_mouse_up(button, MouseButton::Right, Modifiers::none());
    redraw(&mut h.cx);
    assert!(
        h.sidebar
            .read_with(&h.cx, |sidebar, _| sidebar.menu.is_some()),
        "the right click reaches the sidebar"
    );
    assert!(h.cx.debug_bounds("menu-item-project-remove").is_some());
    let setup = center(&mut h, "menu-item-project-setup");
    h.cx.simulate_click(setup, Modifiers::none());
    redraw(&mut h.cx);
    assert!(
        h.sidebar
            .read_with(&h.cx, |sidebar, _| sidebar.setup.is_some())
    );
}

#[gpui_kit::test]
fn continue_with_moves_the_thread_to_another_providers_model(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-continue-with");
    let model = |slug: &str| hoshi_core::ModelInfo {
        slug: slug.into(),
        name: slug.into(),
        short_name: slug.into(),
        legacy: false,
        is_new: false,
        update_required: false,
        efforts: Vec::new(),
        default_effort: None,
        fast_mode: false,
        ultracode: false,
    };
    let catalog = |provider: String, slug: &str| hoshi_core::ProviderModels {
        provider,
        models: vec![model(slug)],
        ..Default::default()
    };
    let open_menu = |h: &mut Harness| {
        let row = center(h, "thread-a");
        h.cx.simulate_mouse_down(row, MouseButton::Right, Modifiers::none());
        h.cx.simulate_mouse_up(row, MouseButton::Right, Modifiers::none());
        redraw(&mut h.cx);
    };
    open_menu(&mut h);
    assert!(
        h.cx.debug_bounds("menu-item-thread-continue-with")
            .is_none(),
        "no other provider, no switch"
    );
    h.cx.simulate_keystrokes("escape");
    h.workspace.update(&mut h.cx, |workspace, cx| {
        workspace.models = vec![
            catalog(hoshi_core::default_provider(), "opus"),
            catalog("codex".into(), "gpt"),
        ];
        cx.notify();
    });
    redraw(&mut h.cx);

    open_menu(&mut h);
    let submenu = center(&mut h, "menu-item-thread-continue-with");
    h.cx.simulate_click(submenu, Modifiers::none());
    redraw(&mut h.cx);
    assert!(
        h.cx.debug_bounds("menu-item-continue-with-claudeCode-opus")
            .is_none()
    );
    let pick = center(&mut h, "menu-item-continue-with-codex-gpt");
    h.cx.simulate_click(pick, Modifiers::none());
    wait_for(&mut h, "the thread to move to codex", |h| {
        h.workspace.read_with(&h.cx, |workspace, _| {
            let thread = workspace.model.thread("a").unwrap();
            thread.provider == "codex" && thread.model.as_deref() == Some("gpt")
        })
    });
}

#[gpui_kit::test]
fn alt_down_steps_through_the_listed_threads(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-step");
    let open = |h: &mut Harness| {
        h.workspace
            .read_with(&h.cx, |workspace, cx| workspace.open_thread_id(cx))
    };
    h.cx.simulate_keystrokes("alt-down");
    redraw(&mut h.cx);
    let first = open(&mut h).expect("a thread opens");
    h.cx.simulate_keystrokes("alt-down");
    redraw(&mut h.cx);
    let second = open(&mut h).expect("the next thread opens");
    assert_ne!(first, second);
    h.cx.simulate_keystrokes("alt-down");
    redraw(&mut h.cx);
    assert_eq!(open(&mut h), Some(first));
}

fn add_threads(h: &mut Harness, count: usize) {
    let engine = h
        .workspace
        .read_with(&h.cx, |workspace, _| workspace.engine.clone());
    for index in 0..count {
        engine
            .dispatch(thread(&format!("x{index}"), &format!("thread {index}")))
            .unwrap();
    }
    let last = format!("x{}", count - 1);
    wait_for(h, "the threads", move |h| {
        h.workspace.read_with(&h.cx, |workspace, _| {
            workspace.model.thread(&last).is_some()
        })
    });
}

fn listed(h: &mut Harness) -> Vec<String> {
    h.sidebar.update(&mut h.cx, |sidebar, cx| {
        sidebar
            .current_sections(cx)
            .into_iter()
            .flat_map(|section| section.threads)
            .map(|thread| thread.id)
            .collect()
    })
}

fn row_bounds(h: &mut Harness, thread_id: &str) -> Option<Bounds<Pixels>> {
    let selector: &'static str = Box::leak(format!("thread-{thread_id}").into_boxed_str());
    h.cx.debug_bounds(selector)
}

fn scroll_list(h: &mut Harness, by: f32) {
    let window = h.cx.update(|window, _| window.viewport_size());
    let at = listed(h)
        .iter()
        .filter_map(|id| row_bounds(h, id))
        .find(|bounds| bounds.top() >= px(0.) && bounds.bottom() <= window.height)
        .expect("a row in view")
        .center();
    h.cx.simulate_mouse_move(at, None, Modifiers::none());
    redraw(&mut h.cx);
    h.cx.simulate_event(gpui_kit::ScrollEvent {
        position: at,
        delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(by))),
        ..Default::default()
    });
    redraw(&mut h.cx);
}

#[gpui_kit::test]
fn scrolling_a_long_list_reaches_its_last_thread(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-long-scroll");
    add_threads(&mut h, 120);
    let order = listed(&mut h);
    let last = order[order.len() - 1].clone();
    let window = h.cx.update(|window, _| window.viewport_size());
    assert!(row_bounds(&mut h, &order[0]).is_some());

    scroll_list(&mut h, -100_000.);
    let bottom = row_bounds(&mut h, &last).expect("the last thread is drawn");
    assert!(
        bottom.top() >= px(0.) && bottom.bottom() <= window.height,
        "{bottom:?}"
    );
}

#[gpui_kit::test]
fn a_long_section_draws_only_the_rows_in_view(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-long-drawn");
    add_threads(&mut h, 300);
    let order = listed(&mut h);
    let drawn = order
        .iter()
        .filter(|id| row_bounds(&mut h, id).is_some())
        .count();
    assert!(drawn > 5, "the rows in view are drawn: {drawn}");
    assert!(drawn < 100, "only the rows in view are drawn, not {drawn}");
}

#[gpui_kit::test]
fn clicking_a_row_opens_its_thread_even_far_down_the_list(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-long-click");
    add_threads(&mut h, 120);
    let order = listed(&mut h);
    let last = order[order.len() - 1].clone();
    scroll_list(&mut h, -100_000.);
    let at = row_bounds(&mut h, &last).unwrap().center();
    h.cx.simulate_mouse_move(at, None, Modifiers::none());
    h.cx.simulate_click(at, Modifiers::none());
    let wanted = last.clone();
    wait_for(&mut h, "the clicked thread to open", move |h| {
        open_thread_id(h).as_deref() == Some(wanted.as_str())
    });
}

#[gpui_kit::test]
fn a_folded_section_is_remembered(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-prefs");
    let engine = h
        .workspace
        .read_with(&h.cx, |workspace, _| workspace.engine.clone());
    engine
        .dispatch(Command::ThreadPin {
            thread_id: "b".into(),
            pinned: true,
        })
        .unwrap();
    wait_for(&mut h, "the pinned section", |h| {
        h.cx.debug_bounds("section-Active").is_some()
    });
    let header = center(&mut h, "section-Active");
    h.cx.simulate_click(header, Modifiers::none());
    h.cx.executor().advance_clock(Duration::from_secs(1));
    let home = h.home.clone();
    wait_for(&mut h, "the prefs to be saved", move |_| {
        prefs::load(&home).collapsed.contains(&SectionId::Active)
    });
    assert!(h.cx.debug_bounds("thread-a").is_none());
}

#[gpui_kit::test]
fn the_rail_and_the_list_fill_the_sidebar_and_scroll_what_overflows(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-overflow");
    let engine = h
        .workspace
        .read_with(&h.cx, |workspace, _| workspace.engine.clone());
    for index in 0..40 {
        engine
            .dispatch(thread(&format!("x{index}"), &format!("t{index}")))
            .unwrap();
        let root = h.home.parent().unwrap().join(format!("project-{index}"));
        std::fs::create_dir_all(&root).unwrap();
        engine
            .dispatch(Command::ProjectCreate {
                project_id: format!("q{index}"),
                title: format!("project {index}"),
                workspace_root: root.to_string_lossy().into_owned(),
            })
            .unwrap();
    }
    wait_for(&mut h, "the threads and projects", |h| {
        h.cx.debug_bounds("thread-x39").is_some() && h.cx.debug_bounds("rail-q39").is_some()
    });
    let (list, rail) = h.sidebar.read_with(&h.cx, |sidebar, _| {
        (
            sidebar.thread_list.max_offset_for_scrollbar().y,
            sidebar.rail_scroll.max_offset().y,
        )
    });
    assert!(list > px(0.), "the list scrolls");
    assert!(rail > px(0.), "the rail scrolls");
    let width = h.sidebar.read_with(&h.cx, |sidebar, _| sidebar.prefs.width);
    let row = h.cx.debug_bounds("thread-x39").unwrap().size.width;
    assert!(row > px(width / 2.), "a short row spans the list: {row:?}");
}

#[gpui_kit::test]
fn dragging_a_project_onto_another_reorders_the_rail(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-drag");
    let engine = h
        .workspace
        .read_with(&h.cx, |workspace, _| workspace.engine.clone());
    let other = h.home.parent().unwrap().join("other");
    std::fs::create_dir_all(&other).unwrap();
    engine
        .dispatch(Command::ProjectCreate {
            project_id: "q".into(),
            title: "other".into(),
            workspace_root: other.to_string_lossy().into_owned(),
        })
        .unwrap();
    wait_for(&mut h, "the second project", |h| {
        h.cx.debug_bounds("rail-q").is_some()
    });
    let (from, to) = (center(&mut h, "rail-q"), center(&mut h, "rail-p"));
    h.cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    for step in 1..=5 {
        let y = from.y + (to.y - from.y) * (step as f32 / 5.);
        h.cx.simulate_mouse_move(Point::new(from.x, y), MouseButton::Left, Modifiers::none());
        redraw(&mut h.cx);
    }
    h.cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    redraw(&mut h.cx);
    let order = h
        .sidebar
        .read_with(&h.cx, |sidebar, _| sidebar.prefs.project_order.clone());
    assert_eq!(order, ["q", "p"]);
    assert!(
        h.cx.debug_bounds("rail-q").unwrap().top() < h.cx.debug_bounds("rail-p").unwrap().top()
    );
}

#[gpui_kit::test]
fn the_card_follows_the_pointer_between_rows_and_closes_when_it_leaves(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-card");
    let shown = |h: &mut Harness| {
        h.sidebar
            .read_with(&h.cx, |sidebar, _| sidebar.cards.shown.clone())
    };
    let (a, b) = (center(&mut h, "thread-a"), center(&mut h, "thread-b"));
    h.cx.simulate_mouse_move(a, None, Modifiers::none());
    redraw(&mut h.cx);
    assert_eq!(shown(&mut h), None);
    h.cx.executor().advance_clock(Duration::from_millis(500));
    redraw(&mut h.cx);
    assert_eq!(shown(&mut h), Some(CardTarget::Thread("a".into())));
    assert!(h.cx.debug_bounds("sidebar-card").is_some());

    h.cx.simulate_mouse_move(b, None, Modifiers::none());
    redraw(&mut h.cx);
    assert_eq!(shown(&mut h), Some(CardTarget::Thread("b".into())));

    let away = center(&mut h, "main");
    h.cx.simulate_mouse_move(away, None, Modifiers::none());
    h.cx.executor().advance_clock(Duration::from_millis(200));
    redraw(&mut h.cx);
    assert_eq!(shown(&mut h), None);
    assert!(h.cx.debug_bounds("sidebar-card").is_none());

    h.cx.simulate_mouse_move(a, None, Modifiers::none());
    h.cx.executor().advance_clock(Duration::from_millis(500));
    redraw(&mut h.cx);
    assert_eq!(shown(&mut h), Some(CardTarget::Thread("a".into())));
    let sliding = h
        .sidebar
        .read_with(&h.cx, |sidebar, _| sidebar.cards.slide.is_some());
    assert!(!sliding, "a card that opens anew appears in place");
}

#[gpui_kit::test]
fn a_new_threads_draft_is_listed_until_discarded(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-drafts");
    h.workspace.update(&mut h.cx, |workspace, cx| {
        workspace.open_thread("a", cx);
        workspace.set_draft(
            "new:p",
            crate::drafts::Draft::from_text("half a thought"),
            cx,
        );
    });
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("draft-row-new:p").is_some());
    h.workspace.update(&mut h.cx, |workspace, cx| {
        workspace.forget_drafts(&["new:p".to_owned()], cx)
    });
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("draft-row-new:p").is_none());
}

#[gpui_kit::test]
fn picking_a_project_on_the_rail_leaves_the_open_thread_for_its_composer(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-pick");
    h.workspace
        .update(&mut h.cx, |workspace, cx| workspace.open_thread("a", cx));
    redraw(&mut h.cx);
    let button = center(&mut h, "rail-p");
    h.cx.simulate_click(button, Modifiers::none());
    redraw(&mut h.cx);
    let (open, draft) = h.workspace.read_with(&h.cx, |workspace, cx| {
        (
            workspace.open_thread_id(cx),
            workspace
                .draft
                .as_ref()
                .map(|draft| draft.project_id.clone()),
        )
    });
    assert_eq!(open, None);
    assert_eq!(draft, Some(Some("p".to_owned())));
}

fn snooze(h: &mut Harness, thread_id: &str, until: chrono::DateTime<chrono::Utc>) {
    let engine = h
        .workspace
        .read_with(&h.cx, |workspace, _| workspace.engine.clone());
    engine
        .dispatch(Command::ThreadSnooze {
            thread_id: thread_id.into(),
            until: until.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        })
        .unwrap();
}

fn snoozed(h: &mut Harness, thread_id: &str) -> bool {
    h.workspace.read_with(&h.cx, |workspace, _| {
        let thread = workspace.model.thread(thread_id).unwrap();
        hoshi_core::is_snoozed(&workspace.model, thread, chrono::Utc::now())
    })
}

fn right_click(h: &mut Harness, selector: &'static str) {
    let at = center(h, selector);
    h.cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
    h.cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
    redraw(&mut h.cx);
}

fn click(h: &mut Harness, selector: &'static str) {
    let at = center(h, selector);
    h.cx.simulate_mouse_move(at, None, Modifiers::none());
    redraw(&mut h.cx);
    h.cx.simulate_click(at, Modifiers::none());
    redraw(&mut h.cx);
}

#[gpui_kit::test]
fn snoozing_the_open_thread_from_its_menu_shelves_it_and_opens_the_next(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-snooze");
    h.workspace
        .update(&mut h.cx, |workspace, cx| workspace.open_thread("a", cx));
    redraw(&mut h.cx);
    right_click(&mut h, "thread-a");
    click(&mut h, "menu-item-thread-snooze");
    assert!(h.cx.debug_bounds("menu-item-snooze-custom").is_some());
    click(&mut h, "menu-item-snooze-after:60");
    wait_for(&mut h, "the thread to be snoozed", |h| snoozed(h, "a"));
    wait_for(&mut h, "the next thread to open", |h| {
        h.workspace
            .read_with(&h.cx, |workspace, cx| workspace.open_thread_id(cx))
            .as_deref()
            == Some("b")
    });
    assert!(h.cx.debug_bounds("section-Snoozed").is_some());

    right_click(&mut h, "thread-a");
    assert!(h.cx.debug_bounds("menu-item-thread-snooze").is_none());
    click(&mut h, "menu-item-thread-wake");
    wait_for(&mut h, "the thread to wake", |h| !snoozed(h, "a"));
}

#[gpui_kit::test]
fn a_snoozed_row_wakes_from_its_hover_button(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-wake");
    snooze(&mut h, "a", chrono::Utc::now() + chrono::Duration::hours(3));
    wait_for(&mut h, "the snoozed section", |h| {
        h.cx.debug_bounds("section-Snoozed").is_some()
    });
    click(&mut h, "snooze-row-a");
    wait_for(&mut h, "the thread to wake", |h| !snoozed(h, "a"));
    redraw(&mut h.cx);
    assert!(h.cx.debug_bounds("section-Snoozed").is_none());
}

#[gpui_kit::test]
fn the_clock_on_a_row_offers_a_custom_snooze(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-custom-snooze");
    click(&mut h, "snooze-row-a");
    click(&mut h, "menu-item-snooze-custom");
    let form = h
        .sidebar
        .read_with(&h.cx, |sidebar, _| {
            sidebar.custom_snooze.as_ref().map(|(form, _)| form.clone())
        })
        .expect("the custom snooze dialog opens");
    let submit = |h: &mut Harness, amount: &str| {
        let amount = amount.to_owned();
        h.cx.update(|window, cx| {
            form.update(cx, |form, cx| {
                form.type_duration(&amount, window, cx);
                form.submit(cx);
            })
        });
        redraw(&mut h.cx);
    };
    submit(&mut h, "0");
    let error = form.read_with(&h.cx, |form, _| form.error().map(str::to_owned));
    assert_eq!(error.as_deref(), Some("Enter a positive duration."));
    assert!(h.cx.debug_bounds("custom-snooze-error").is_some());
    submit(&mut h, "2");
    wait_for(&mut h, "the thread to be snoozed", |h| snoozed(h, "a"));
    assert!(
        h.sidebar
            .read_with(&h.cx, |sidebar, _| sidebar.custom_snooze.is_none())
    );
}

#[gpui_kit::test]
fn a_fork_is_marked_in_its_row_and_its_source_is_not(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-fork-mark");
    let source = h.workspace.read_with(&h.cx, |workspace, _| {
        workspace.model.thread("a").unwrap().clone()
    });
    let fork = hoshi_core::Thread {
        id: "f".into(),
        title: "first fork".into(),
        forked_from: Some(Box::new(hoshi_core::ForkOrigin {
            thread_id: "a".into(),
            turn: hoshi_core::TurnPoint {
                turn_id: "m1".into(),
                through_item_id: "a1".into(),
                ordinal: 1,
                status: hoshi_core::TurnStatus::Completed,
                provider: source.provider.clone(),
                session_id: None,
                cursor: None,
            },
        })),
        ..source
    };
    h.workspace.update(&mut h.cx, |workspace, cx| {
        workspace.model.apply(&hoshi_core::Event::ThreadCreated {
            thread: Box::new(fork),
        });
        cx.notify();
    });
    wait_for(&mut h, "the fork's row", |h| {
        h.cx.debug_bounds("thread-f").is_some()
    });
    assert!(h.cx.debug_bounds("fork-mark-f").is_some());
    assert!(h.cx.debug_bounds("fork-mark-a").is_none());
}

#[gpui_kit::test]
fn the_custom_snooze_dialog_keeps_its_place_across_its_tabs(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-custom-snooze-tabs");
    click(&mut h, "snooze-row-a");
    click(&mut h, "menu-item-snooze-custom");
    let form = h
        .sidebar
        .read_with(&h.cx, |sidebar, _| {
            sidebar.custom_snooze.as_ref().map(|(form, _)| form.clone())
        })
        .expect("the custom snooze dialog opens");
    redraw(&mut h.cx);
    let date = h.cx.debug_bounds("custom-snooze-form").expect("the form");
    form.update(&mut h.cx, |form, cx| {
        form.set_mode(super::snooze::Mode::For, cx)
    });
    redraw(&mut h.cx);
    let duration = h.cx.debug_bounds("custom-snooze-form").expect("the form");
    assert_eq!(duration, date);
}

#[gpui_kit::test]
fn the_list_sorts_again_once_a_snooze_ends_and_the_row_says_it_woke(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-snooze-ends");
    let until = chrono::Utc::now() + chrono::Duration::seconds(1);
    snooze(&mut h, "a", until);
    snooze(&mut h, "b", until + chrono::Duration::milliseconds(100));
    wait_for(&mut h, "the snoozed section", |h| {
        h.cx.debug_bounds("section-Snoozed").is_some()
    });
    let scheduled = h.sidebar.read_with(&h.cx, |sidebar, _| {
        sidebar.refresh.as_ref().map(|(at, _)| *at)
    });
    assert_eq!(
        scheduled.map(|at| at.timestamp_millis()),
        Some(until.timestamp_millis())
    );
    let notified = std::rc::Rc::new(std::cell::Cell::new(0));
    let counted = notified.clone();
    let workspace = h.workspace.clone();
    h.cx.update(|_, cx| {
        cx.observe(&workspace, move |_, _| counted.set(counted.get() + 1))
            .detach()
    });
    std::thread::sleep(Duration::from_millis(1200));
    h.cx.executor().advance_clock(Duration::from_secs(2));
    redraw(&mut h.cx);
    assert!(notified.get() > 0, "the end of the snooze redraws the list");
    assert!(
        h.sidebar
            .read_with(&h.cx, |sidebar, _| sidebar.refresh.is_none())
    );
    assert!(h.cx.debug_bounds("section-Snoozed").is_none());
    click(&mut h, "woke-a");
    wait_for(&mut h, "the Woke chip to go", |h| {
        h.cx.debug_bounds("woke-a").is_none()
    });
    assert!(h.cx.debug_bounds("woke-b").is_some());
    h.workspace
        .update(&mut h.cx, |workspace, cx| workspace.open_thread("b", cx));
    wait_for(&mut h, "the visit to put the chip away", |h| {
        h.cx.debug_bounds("woke-b").is_none()
    });
}

#[gpui_kit::test]
fn project_settings_override_continuing_after_restarts(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-project-settings");
    right_click(&mut h, "rail-p");
    click(&mut h, "menu-item-project-settings");
    assert!(h.cx.debug_bounds("project-continue-source").is_none());
    click(&mut h, "select-trigger");
    click(&mut h, "menu-item-project-continue-off");
    wait_for(&mut h, "the project's own value", |h| {
        h.workspace.read_with(&h.cx, |workspace, _| {
            workspace
                .model
                .project("p")
                .unwrap()
                .settings
                .continue_after_restart
                == Some(false)
        })
    });
    let effective = h.workspace.read_with(&h.cx, |workspace, _| {
        workspace.engine.continue_after_restart_in(Some("p"))
    });
    assert!(!effective.value);
    assert_eq!(effective.source, hoshi_core::SettingSource::Project);
    wait_for(&mut h, "the project's own value to be named", |h| {
        h.cx.debug_bounds("project-continue-source").is_some()
    });

    click(&mut h, "project-worktree-setup-edit");
    let (settings, setup) = h.sidebar.read_with(&h.cx, |sidebar, _| {
        (sidebar.project_settings.clone(), sidebar.setup.is_some())
    });
    assert_eq!(settings, None);
    assert!(setup);
}

fn open_thread_id(h: &mut Harness) -> Option<String> {
    h.workspace
        .read_with(&h.cx, |workspace, cx| workspace.open_thread_id(cx))
}

#[gpui_kit::test]
fn undo_wakes_the_snoozed_thread_and_opens_it_again(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-snooze-undo");
    h.workspace
        .update(&mut h.cx, |workspace, cx| workspace.open_thread("a", cx));
    redraw(&mut h.cx);
    right_click(&mut h, "thread-a");
    click(&mut h, "menu-item-thread-snooze");
    click(&mut h, "menu-item-snooze-after:60");
    wait_for(&mut h, "the next thread to open", |h| {
        open_thread_id(h).as_deref() == Some("b") && h.cx.debug_bounds("toast").is_some()
    });
    assert!(snoozed(&mut h, "a"));

    click(&mut h, "toast-action");
    wait_for(&mut h, "the thread to wake and open", |h| {
        !snoozed(h, "a") && open_thread_id(h).as_deref() == Some("a")
    });
    assert!(h.cx.debug_bounds("toast").is_none());
}

#[gpui_kit::test]
fn ctrl_z_undoes_a_snooze_from_the_rows_clock(cx: &mut TestAppContext) {
    let mut h = harness(cx, "sidebar-snooze-undo-key");
    click(&mut h, "snooze-row-a");
    click(&mut h, "menu-item-snooze-after:60");
    wait_for(&mut h, "the notice", |h| {
        snoozed(h, "a") && h.cx.debug_bounds("toast").is_some()
    });
    h.cx.simulate_keystrokes("ctrl-z");
    wait_for(&mut h, "the thread to wake", |h| !snoozed(h, "a"));
    assert_eq!(
        open_thread_id(&mut h),
        None,
        "it was not open, so it stays closed"
    );
    assert!(h.cx.debug_bounds("toast").is_none());
}

#[gpui_kit::test]
fn the_custom_snooze_calendar_picks_a_day_ahead(cx: &mut TestAppContext) {
    use chrono::{Days, Local, TimeZone};
    let mut h = harness(cx, "sidebar-snooze-calendar");
    click(&mut h, "snooze-row-a");
    click(&mut h, "menu-item-snooze-custom");
    let form = h
        .sidebar
        .read_with(&h.cx, |sidebar, _| {
            sidebar.custom_snooze.as_ref().map(|(form, _)| form.clone())
        })
        .expect("the custom snooze dialog opens");
    let today = Local::now().date_naive();
    let tomorrow = today + Days::new(1);
    assert_eq!(form.read_with(&h.cx, |form, _| form.picked_day()), tomorrow);
    assert!(h.cx.debug_bounds("custom-snooze-resolved").is_some());

    let cell = |day: chrono::NaiveDate| -> &'static str {
        format!("custom-snooze-calendar-day-{}", day.format("%Y-%m-%d")).leak()
    };
    let yesterday = today - Days::new(1);
    if h.cx.debug_bounds(cell(yesterday)).is_some() {
        click(&mut h, cell(yesterday));
        assert_eq!(form.read_with(&h.cx, |form, _| form.picked_day()), tomorrow);
    }
    let target = today + Days::new(3);
    if crate::calendar::month_of(target) != crate::calendar::month_of(tomorrow) {
        click(&mut h, "custom-snooze-calendar-next");
    }
    click(&mut h, cell(target));
    assert_eq!(form.read_with(&h.cx, |form, _| form.picked_day()), target);

    h.cx.update(|_, cx| form.update(cx, |form, cx| form.submit(cx)));
    wait_for(&mut h, "the thread to be snoozed", |h| snoozed(h, "a"));
    let until = h.workspace.read_with(&h.cx, |workspace, _| {
        workspace.model.thread("a").unwrap().snoozed_until.clone()
    });
    let expected = Local
        .from_local_datetime(&target.and_hms_opt(9, 0, 0).unwrap())
        .earliest()
        .unwrap();
    let until = chrono::DateTime::parse_from_rfc3339(&until.unwrap()).unwrap();
    assert_eq!(until, expected);
}
