use super::{MiniBrowser, Obstacle, obstacle};
use crate::browser::layout::{Interaction, Placement};
use crate::panel::{RightPanel, SurfaceKind};
use crate::state::{self, Workspace};
use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, IntoElement, Modifiers, MouseButton,
    ParentElement as _, Pixels, Point, Render, Styled as _, TestAppContext, VisualTestContext,
    Window, WindowBounds, WindowOptions, div, point, px, size,
};
use hoshi_core::{AccessMode, BrowserPage, Command, ThreadWorkspace};
use std::sync::Arc;

const COMPOSER: f32 = 120.;

/// The chat column: the transcript, the composer at the bottom, the player over them.
struct Chat {
    mini: Entity<MiniBrowser>,
}

impl Render for Chat {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1())
            .child(obstacle(Obstacle::Composer))
            .child(div().flex_none().h(px(COMPOSER)))
            .child(self.mini.clone())
    }
}

struct Harness {
    cx: VisualTestContext,
    workspace: Entity<Workspace>,
    panel: Entity<RightPanel>,
    mini: Entity<MiniBrowser>,
}

impl Harness {
    fn browsers(&self) -> Entity<crate::browser::Browsers> {
        self.workspace
            .read_with(&self.cx, |workspace, _| workspace.browsers.clone())
    }

    fn player(&mut self) -> Option<Bounds<Pixels>> {
        self.cx.debug_bounds("mini-browser")
    }

    fn viewers(&mut self) -> usize {
        let browsers = self.browsers();
        browsers.read_with(&self.cx, |browsers, _| browsers.viewers("t"))
    }

    fn placement(&mut self) -> Option<Placement> {
        let browsers = self.browsers();
        browsers.read_with(&self.cx, |browsers, _| browsers.mini("t"))
    }

    fn drag(&mut self, from: Point<Pixels>, to: Point<Pixels>) {
        self.cx
            .simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
        redraw(&mut self.cx);
        let middle = point((from.x + to.x) / 2., (from.y + to.y) / 2.);
        for at in [middle, to] {
            self.cx
                .simulate_mouse_move(at, Some(MouseButton::Left), Modifiers::default());
            redraw(&mut self.cx);
        }
        self.cx
            .simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
        redraw(&mut self.cx);
    }

    fn hover_handle(&mut self) -> Bounds<Pixels> {
        self.cx
            .simulate_mouse_move(point(px(1.), px(1.)), None, Modifiers::default());
        redraw(&mut self.cx);
        let dot = self.cx.debug_bounds("mini-browser-handle").unwrap();
        self.cx
            .simulate_mouse_move(dot.center(), None, Modifiers::default());
        redraw(&mut self.cx);
        let pill = self.cx.debug_bounds("mini-browser-handle").unwrap();
        assert!(pill.size.width > dot.size.width, "hovering shows the pill");
        pill
    }
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
    }
}

fn agent_page() -> BrowserPage {
    BrowserPage {
        thread_id: "t".into(),
        open: true,
        url: "http://localhost:5173/".into(),
        title: "app".into(),
        errors: 0,
        by_agent: true,
        agent_point: None,
        opened: true,
        fitted: Some(true),
    }
}

fn harness(width: f32, height: f32, cx: &mut TestAppContext) -> Harness {
    cx.executor().allow_parking();
    let home = std::env::temp_dir().join(format!("hoshi-native-mini-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
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
    });
    let workspace = cx.update(|cx| cx.new(|cx| Workspace::new(engine, notifications, cx)));
    workspace.update(cx, |workspace, cx| {
        workspace.settings.browser.float_agent_pages = true;
        workspace.open_thread("t", cx)
    });
    let mut parts = None;
    let (window, _) = cx
        .update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(width), px(height)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                let panel = cx.new(|cx| RightPanel::new(workspace.clone(), window, cx));
                let mini = cx.new(|cx| MiniBrowser::new(workspace.clone(), panel.clone(), cx));
                parts = Some((panel, mini.clone()));
                cx.new(|_| Chat { mini })
            })
        })
        .unwrap();
    let (panel, mini) = parts.unwrap();
    let mut cx = VisualTestContext::from_window(window, cx);
    redraw(&mut cx);
    Harness {
        cx,
        workspace,
        panel,
        mini,
    }
}

fn receive(h: &mut Harness, page: BrowserPage) {
    let browsers = h.browsers();
    browsers.update(&mut h.cx, |browsers, cx| browsers.receive_page(page, cx));
    redraw(&mut h.cx);
}

#[gpui_kit::test]
fn an_agent_page_floats_in_the_corner_moves_the_chat_aside_and_streams(cx: &mut TestAppContext) {
    let mut h = harness(1400., 800., cx);
    assert!(h.player().is_none());
    assert_eq!(h.viewers(), 0);

    receive(&mut h, agent_page());
    let player = h.player().expect("the page floats");
    assert_eq!(player.size, size(px(320.), px(200.)));
    assert_eq!(player.right(), px(1400. - 12.));
    assert_eq!(player.bottom(), px(800. - 12.));
    assert!(h.cx.debug_bounds("mini-browser-page").is_some());
    assert_eq!(h.viewers(), 1);
    let room = h.mini.read_with(&h.cx, |mini, _| mini.lane_room());
    let room = room.expect("the chat moves aside");
    assert!(room.right >= px(1400. - 1068. + 12.), "{room:?}");

    receive(
        &mut h,
        BrowserPage {
            open: false,
            ..agent_page()
        },
    );
    assert!(h.player().is_none());
    assert_eq!(h.viewers(), 0);
    assert!(
        h.mini
            .read_with(&h.cx, |mini, _| mini.lane_room())
            .is_none()
    );
}

#[gpui_kit::test]
fn in_a_narrow_chat_it_floats_above_the_composer(cx: &mut TestAppContext) {
    let mut h = harness(1000., 700., cx);
    receive(&mut h, agent_page());
    let player = h.player().expect("the page floats");
    assert!(player.bottom() <= px(700. - COMPOSER - 12.), "{player:?}");
    assert!(
        h.mini
            .read_with(&h.cx, |mini, _| mini.lane_room())
            .is_none()
    );
}

#[gpui_kit::test]
fn the_handle_moves_it_past_a_tap_and_an_edge_resizes_it_at_the_page_shape(
    cx: &mut TestAppContext,
) {
    let mut h = harness(1400., 800., cx);
    receive(&mut h, agent_page());
    let handle = h.cx.debug_bounds("mini-browser-handle").unwrap().center();

    h.drag(handle, handle + point(px(3.), px(2.)));
    assert_eq!(
        h.placement().unwrap().position,
        None,
        "a tap doesn't move it"
    );

    let before = h.player().unwrap();
    let pill = h.hover_handle();
    let frame = point(pill.left() + px(1.5), pill.center().y);
    h.drag(frame, frame - point(px(500.), px(300.)));
    assert_eq!(h.placement().unwrap().interaction, Interaction::Drag);
    let player = h.player().unwrap();
    assert_eq!(player.origin.y, before.origin.y - px(300.));
    assert_eq!(
        player.origin.x,
        px(48. + 640. + 12.),
        "the drag stops where the chat turns unreadable"
    );

    let edge = point(player.right(), player.center().y);
    h.drag(edge, edge + point(px(160.), px(0.)));
    let placement = h.placement().unwrap();
    assert_eq!(placement.interaction, Interaction::Resize);
    assert_eq!(placement.width, Some(480.));
    let resized = h.player().unwrap();
    assert_eq!(resized.size, size(px(480.), px(300.)));
    assert_eq!(resized.origin, player.origin);
}

#[gpui_kit::test]
fn a_deleted_thread_takes_its_floating_page_with_it(cx: &mut TestAppContext) {
    let mut h = harness(1400., 800., cx);
    receive(&mut h, agent_page());
    assert!(h.placement().is_some());

    let browsers = h.browsers();
    browsers.update(&mut h.cx, |browsers, cx| browsers.forget("t", cx));
    redraw(&mut h.cx);
    assert!(h.placement().is_none());
    assert!(h.player().is_none());
    assert_eq!(h.viewers(), 0);
}

#[gpui_kit::test]
fn the_pill_sends_it_to_the_panel_or_closes_it(cx: &mut TestAppContext) {
    let mut h = harness(1400., 800., cx);
    receive(&mut h, agent_page());
    let pill = h.hover_handle();
    h.cx.simulate_click(
        point(pill.left() + px(12.), pill.center().y),
        Modifiers::default(),
    );
    redraw(&mut h.cx);
    assert!(h.placement().is_none());
    assert!(h.player().is_none());
    assert!(
        h.panel
            .read_with(&h.cx, |panel, _| panel.shows_browser("t"))
    );

    let browsers = h.browsers();
    browsers.update(&mut h.cx, |browsers, cx| browsers.float("t", cx));
    redraw(&mut h.cx);
    assert!(!h.panel.read_with(&h.cx, |panel, _| panel.is_open("t")));
    assert!(h.player().is_some());

    h.panel.update(&mut h.cx, |panel, cx| {
        panel.open_surface("t", SurfaceKind::Diff, cx);
        panel.toggle_maximized(cx);
    });
    redraw(&mut h.cx);
    assert!(h.player().is_none(), "the chat is hidden");
    assert_eq!(h.viewers(), 0);
    h.panel.update(&mut h.cx, |panel, cx| panel.close("t", cx));
    redraw(&mut h.cx);
    assert!(h.player().is_some());

    let pill = h.hover_handle();
    h.cx.simulate_click(
        point(pill.right() - px(12.), pill.center().y),
        Modifiers::default(),
    );
    redraw(&mut h.cx);
    assert!(h.placement().is_none());
    assert!(h.player().is_none());
    assert!(!h.panel.read_with(&h.cx, |panel, _| panel.is_open("t")));
    assert_eq!(h.viewers(), 0);
}
