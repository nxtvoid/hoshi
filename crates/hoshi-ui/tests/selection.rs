//! Where a drag may start a text selection: on selectable text or inside a selection area,
//! never from a clickable elsewhere, a popover's trigger or what covers an area.

use gpui_kit::base::TextSelection;
use gpui_kit::component::text::TextView;
use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{
    AppContext as _, Bounds, Context, InteractiveElement as _, IntoElement, Modifiers, MouseButton,
    ParentElement as _, Pixels, Point, Render, StatefulInteractiveElement as _, Styled as _,
    TestAppContext, VisualTestContext, Window, WindowBounds, WindowOptions, div, point, px, size,
};
use hoshi_ui::{Button, Popover, selection_area, selection_release};

const TEXT: &str = "First line of selectable text.\n\nSecond line of selectable text.";

struct Page;

impl Render for Page {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(selection_release())
            .child(
                div()
                    .flex()
                    .h(px(40.))
                    .child(
                        div()
                            .id("plain-clickable")
                            .w(px(100.))
                            .h_full()
                            .on_click(|_, _, _| {}),
                    )
                    .child(
                        Popover::new("popover")
                            .trigger(|_| Button::new("trigger").label("Open"))
                            .content(|_, _, _| div().child("Inside")),
                    ),
            )
            .child(
                div()
                    .relative()
                    .h(px(200.))
                    .pl(px(100.))
                    .child(selection_area())
                    .child(TextView::markdown("text", TEXT).selectable(true))
                    .child(
                        div()
                            .id("cover")
                            .absolute()
                            .top_0()
                            .left_0()
                            .w(px(40.))
                            .h(px(40.))
                            .occlude(),
                    ),
            )
    }
}

fn open(cx: &mut TestAppContext) -> VisualTestContext {
    cx.update(|cx| {
        gpui_kit::init(cx);
        hoshi_ui::theme::apply(ThemeMode::Dark, cx);
    });
    let window = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(600.), px(400.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Page))
            .expect("test window")
            .0
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    redraw(&mut cx);
    cx
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    for _ in 0..3 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
}

/// Presses at `from`, drags across the text and lets go, then moves on with no button down,
/// as after a click.
fn drag_from(cx: &mut VisualTestContext, from: Point<Pixels>) -> String {
    let over_text = point(px(300.), px(60.));
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(over_text, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(over_text, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(point(px(400.), px(80.)), None, Modifiers::default());
    redraw(cx);
    cx.update(TextSelection::selected_text)
}

#[gpui_kit::test]
fn a_drag_from_the_text_selects_it(cx: &mut TestAppContext) {
    let mut cx = open(cx);
    let selected = drag_from(&mut cx, point(px(102.), px(48.)));
    assert!(selected.starts_with("First"), "selected: {selected:?}");
}

#[gpui_kit::test]
fn a_drag_from_the_margin_of_an_area_selects(cx: &mut TestAppContext) {
    let mut cx = open(cx);
    let selected = drag_from(&mut cx, point(px(70.), px(48.)));
    assert!(!selected.is_empty());
}

#[gpui_kit::test]
fn a_press_on_a_clickable_outside_an_area_selects_nothing(cx: &mut TestAppContext) {
    let mut cx = open(cx);
    let selected = drag_from(&mut cx, point(px(50.), px(20.)));
    assert_eq!(selected, "");
}

#[gpui_kit::test]
fn a_press_on_a_popover_trigger_selects_nothing(cx: &mut TestAppContext) {
    let mut cx = open(cx);
    let trigger = cx.debug_bounds("popover-trigger");
    let at = trigger.map_or(point(px(130.), px(20.)), |bounds| bounds.center());
    let selected = drag_from(&mut cx, at);
    assert_eq!(selected, "");
}

#[gpui_kit::test]
fn what_covers_an_area_is_not_part_of_it(cx: &mut TestAppContext) {
    let mut cx = open(cx);
    let selected = drag_from(&mut cx, point(px(20.), px(60.)));
    assert_eq!(selected, "");
}
