//! The select in a headless window: its list is at least as wide as its trigger.

use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{
    AppContext as _, Bounds, Context, IntoElement, Modifiers, MouseButton, ParentElement as _,
    Point, Render, Styled as _, TestAppContext, VisualTestContext, Window, WindowBounds,
    WindowOptions, div, px, size,
};
use hoshi_ui::{Select, SelectOption};

struct Harness;

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().flex().p_4().child(
            Select::new("model", "Default")
                .min_width(px(300.))
                .option(SelectOption::new("default", "Default", true))
                .option(SelectOption::new("short", "Short", false)),
        )
    }
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    for _ in 0..2 {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
}

#[gpui_kit::test]
fn the_list_is_as_wide_as_the_trigger(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        hoshi_ui::theme::apply(ThemeMode::Light, cx);
        cx.set_reduce_motion(true);
    });
    let window = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(800.), px(400.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Harness))
            .expect("test window")
            .0
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    redraw(&mut cx);

    let trigger = cx
        .debug_bounds("select-trigger")
        .expect("the trigger shows");
    let at = trigger.center();
    cx.simulate_mouse_move(at, None, Modifiers::default());
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
    redraw(&mut cx);
    cx.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
    redraw(&mut cx);

    let list = cx.debug_bounds("popover-popup").expect("the list opens");
    assert_eq!(list.size.width, trigger.size.width);
    assert_eq!(list.left(), trigger.left());
}
