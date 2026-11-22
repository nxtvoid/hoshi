use super::{Tooltip, TooltipTrigger, WithTooltip};
use crate::{Button, Side};
use gpui_kit::component::theme::ThemeMode;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Bounds, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    Modifiers, MouseButton, ParentElement as _, Pixels, Point, Render,
    StatefulInteractiveElement as _, Styled as _, TestAppContext, VisualTestContext, Window,
    WindowBounds, WindowOptions, div, point, px, size,
};
use std::time::Duration;

const LONG: &str = "A tooltip long enough that it has to wrap onto more than one line \
                        because the popup stops growing at twenty rems";

struct Triggers {
    first: bool,
    focus: FocusHandle,
    clicked: bool,
}

impl Render for Triggers {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let at = |x: f32, y: f32| div().absolute().left(px(x)).top(px(y));
        div()
            .size_full()
            .when(self.first, |this| {
                this.child(
                    at(100., 100.).child(Button::new("first").label("First").tooltip("Alpha")),
                )
            })
            .child(
                at(300., 100.).child(
                    Button::new("second")
                        .label("Second")
                        .tooltip(Tooltip::new("Beta").side(Side::Bottom).shortcut("Ctrl K")),
                ),
            )
            .child(at(500., 100.).child(Button::new("long").label("Long").tooltip(LONG)))
            .child(
                at(100., 300.).child(
                    Button::new("off")
                        .label("Off")
                        .tooltip(Tooltip::new("Never").disabled(true)),
                ),
            )
            .child(
                at(300., 300.).child(
                    div()
                        .id("focusable")
                        .track_focus(&self.focus)
                        .size(px(40.))
                        .with_tooltip(Tooltip::new("Focused").focus(&self.focus)),
                ),
            )
            .child(
                at(300., 270.).child(
                    div()
                        .id("under")
                        .w(px(40.))
                        .h(px(20.))
                        .on_click(cx.listener(|this, _, _, _| this.clicked = true)),
                ),
            )
            .child(
                at(500., 300.).child(TooltipTrigger::new(
                    "custom",
                    div().size(px(40.)),
                    Tooltip::element(|_, _| div().flex().flex_col().child("Title").child("Path"))
                        .side(Side::Right)
                        .side_offset(px(12.)),
                )),
            )
    }
}

const FIRST: Point<Pixels> = point(px(110.), px(110.));
const SECOND: Point<Pixels> = point(px(310.), px(110.));
const LONG_TRIGGER: Point<Pixels> = point(px(510.), px(110.));
const OFF: Point<Pixels> = point(px(110.), px(310.));
const CUSTOM: Point<Pixels> = point(px(510.), px(310.));
const NOWHERE: Point<Pixels> = point(px(700.), px(380.));

fn open(cx: &mut TestAppContext) -> (VisualTestContext, Entity<Triggers>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply(ThemeMode::Light, cx);
    });
    let (window, view) = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(800.), px(400.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| {
            cx.new(|cx| Triggers {
                first: true,
                focus: cx.focus_handle(),
                clicked: false,
            })
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

fn hover(cx: &mut VisualTestContext, position: Point<Pixels>) {
    cx.simulate_mouse_move(position, None, Modifiers::default());
    redraw(cx);
}

fn wait(cx: &mut VisualTestContext, millis: u64) {
    cx.executor().advance_clock(Duration::from_millis(millis));
    redraw(cx);
}

fn tooltip(cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
    cx.debug_bounds("tooltip")
}

fn shown_after_delay(cx: &mut VisualTestContext, position: Point<Pixels>) -> Bounds<Pixels> {
    hover(cx, position);
    assert_eq!(tooltip(cx), None, "opens only after the delay");
    wait(cx, 600);
    tooltip(cx).expect("open after the delay")
}

#[gpui_kit::test]
fn a_tooltip_opens_after_the_delay_above_its_trigger(cx: &mut TestAppContext) {
    let (mut cx, _) = open(cx);
    hover(&mut cx, FIRST);
    wait(&mut cx, 599);
    assert_eq!(tooltip(&mut cx), None);
    wait(&mut cx, 1);
    let bounds = tooltip(&mut cx).expect("open after 600ms");
    assert_eq!(
        bounds.bottom(),
        px(96.),
        "4px above the trigger: {bounds:?}"
    );
    assert_eq!(bounds.size.height, px(28.));
}

#[gpui_kit::test]
fn the_next_trigger_opens_at_once_within_the_group_window(cx: &mut TestAppContext) {
    let (mut cx, _) = open(cx);
    shown_after_delay(&mut cx, FIRST);
    hover(&mut cx, SECOND);
    let bounds = tooltip(&mut cx).expect("the second opens at once");
    assert_eq!(bounds.top(), px(136.), "below the trigger: {bounds:?}");

    hover(&mut cx, NOWHERE);
    assert_eq!(tooltip(&mut cx), None);
    wait(&mut cx, 300);
    hover(&mut cx, FIRST);
    assert!(tooltip(&mut cx).is_some(), "still within the group window");

    hover(&mut cx, NOWHERE);
    wait(&mut cx, 401);
    shown_after_delay(&mut cx, SECOND);
}

#[gpui_kit::test]
fn leaving_the_trigger_closes_and_cancels_a_pending_open(cx: &mut TestAppContext) {
    let (mut cx, _) = open(cx);
    shown_after_delay(&mut cx, FIRST);
    hover(&mut cx, NOWHERE);
    assert_eq!(tooltip(&mut cx), None);

    wait(&mut cx, 1_000);
    hover(&mut cx, FIRST);
    wait(&mut cx, 300);
    hover(&mut cx, NOWHERE);
    wait(&mut cx, 600);
    assert_eq!(tooltip(&mut cx), None);
}

#[gpui_kit::test]
fn a_press_closes_and_moving_within_the_trigger_does_not_reopen(cx: &mut TestAppContext) {
    let (mut cx, _) = open(cx);
    shown_after_delay(&mut cx, FIRST);
    cx.simulate_mouse_down(FIRST, MouseButton::Left, Modifiers::default());
    redraw(&mut cx);
    assert_eq!(tooltip(&mut cx), None);
    cx.simulate_mouse_up(FIRST, MouseButton::Left, Modifiers::default());
    hover(&mut cx, point(px(112.), px(112.)));
    wait(&mut cx, 1_000);
    assert_eq!(tooltip(&mut cx), None);

    hover(&mut cx, NOWHERE);
    wait(&mut cx, 1_000);
    hover(&mut cx, FIRST);
    cx.simulate_mouse_down(FIRST, MouseButton::Left, Modifiers::default());
    wait(&mut cx, 1_000);
    assert_eq!(tooltip(&mut cx), None, "a press cancels the pending open");
}

#[gpui_kit::test]
fn escape_closes_the_tooltip(cx: &mut TestAppContext) {
    let (mut cx, _) = open(cx);
    shown_after_delay(&mut cx, FIRST);
    cx.simulate_keystrokes("escape");
    redraw(&mut cx);
    assert_eq!(tooltip(&mut cx), None);
}

#[gpui_kit::test]
fn scrolling_closes_the_tooltip(cx: &mut TestAppContext) {
    let (mut cx, _) = open(cx);
    shown_after_delay(&mut cx, FIRST);
    cx.simulate_event(gpui_kit::ScrollEvent {
        position: FIRST,
        delta: gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-20.))),
        ..Default::default()
    });
    redraw(&mut cx);
    assert_eq!(tooltip(&mut cx), None);
}

#[gpui_kit::test]
fn the_window_going_inactive_closes_the_tooltip(cx: &mut TestAppContext) {
    let (mut cx, _) = open(cx);
    cx.update(|window, _| window.activate_window());
    redraw(&mut cx);
    shown_after_delay(&mut cx, FIRST);
    cx.deactivate_window();
    redraw(&mut cx);
    assert_eq!(tooltip(&mut cx), None);
}

#[gpui_kit::test]
fn a_tooltip_closes_with_its_trigger_and_the_group_resets(cx: &mut TestAppContext) {
    let (mut cx, view) = open(cx);
    shown_after_delay(&mut cx, FIRST);
    view.update(&mut cx, |view, cx| {
        view.first = false;
        cx.notify();
    });
    redraw(&mut cx);
    assert_eq!(tooltip(&mut cx), None);
    shown_after_delay(&mut cx, SECOND);
}

#[gpui_kit::test]
fn long_content_wraps_at_the_max_width(cx: &mut TestAppContext) {
    let (mut cx, _) = open(cx);
    let short = shown_after_delay(&mut cx, FIRST);
    assert!(short.size.width < px(100.), "{short:?}");
    hover(&mut cx, LONG_TRIGGER);
    let long = tooltip(&mut cx).expect("open");
    assert_eq!(long.size.width, px(320.), "{long:?}");
    assert!(long.size.height > px(40.), "{long:?}");
    assert!(long.right() <= px(795.), "kept inside the window: {long:?}");
}

#[gpui_kit::test]
fn custom_content_takes_its_side_and_offset(cx: &mut TestAppContext) {
    let (mut cx, _) = open(cx);
    let bounds = shown_after_delay(&mut cx, CUSTOM);
    assert_eq!(
        bounds.left(),
        px(552.),
        "12px right of the trigger: {bounds:?}"
    );
    assert_eq!(bounds.size.height, px(44.), "two lines: {bounds:?}");
}

#[gpui_kit::test]
fn a_disabled_tooltip_never_opens(cx: &mut TestAppContext) {
    let (mut cx, _) = open(cx);
    hover(&mut cx, OFF);
    wait(&mut cx, 1_000);
    assert_eq!(tooltip(&mut cx), None);
}

#[gpui_kit::test]
fn keyboard_focus_opens_at_once_and_blur_closes(cx: &mut TestAppContext) {
    let (mut cx, view) = open(cx);
    cx.simulate_keystrokes("a");
    let focus = view.read_with(&cx, |view, _| view.focus.clone());
    cx.update(|window, cx| focus.focus(window, cx));
    redraw(&mut cx);
    assert!(tooltip(&mut cx).is_some(), "focus opens without a delay");
    cx.update(|window, cx| window.blur(cx));
    redraw(&mut cx);
    assert_eq!(tooltip(&mut cx), None);
}

#[gpui_kit::test]
fn clicks_reach_what_lies_under_the_tooltip(cx: &mut TestAppContext) {
    let (mut cx, view) = open(cx);
    cx.simulate_keystrokes("a");
    let focus = view.read_with(&cx, |view, _| view.focus.clone());
    cx.update(|window, cx| focus.focus(window, cx));
    redraw(&mut cx);
    let bounds = tooltip(&mut cx).expect("focus opens it");
    let under = point(px(310.), px(280.));
    assert!(bounds.contains(&under), "{bounds:?}");
    cx.simulate_click(under, Modifiers::default());
    redraw(&mut cx);
    assert!(view.read_with(&cx, |view, _| view.clicked));
}
