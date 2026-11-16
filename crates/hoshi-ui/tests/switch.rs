use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{
    AppContext as _, Bounds, Context, IntoElement, ParentElement as _, Point, Render, Styled as _,
    TestAppContext, VisualTestContext, Window, WindowBounds, WindowOptions, div, px, size,
};
use hoshi_ui::Switch;
use hoshi_ui::theme;

struct Screen {
    on: bool,
}

impl Render for Screen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.weak_entity();
        div()
            .size_full()
            .child(
                Switch::new("switch")
                    .checked(self.on)
                    .on_change(move |on, _, cx| {
                        this.update(cx, |this, cx| {
                            this.on = on;
                            cx.notify();
                        })
                        .ok();
                    }),
            )
    }
}

#[gpui_kit::test]
fn tab_reaches_the_switch_and_space_toggles_it(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::apply(ThemeMode::Dark, cx);
    });
    let (window, screen) = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(200.), px(100.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Screen { on: false })).unwrap()
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.run_until_parked();
    cx.update(|window, cx| window.focus_next(cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("space");
    cx.run_until_parked();
    assert!(screen.read_with(&cx, |screen, _| screen.on));
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(!screen.read_with(&cx, |screen, _| screen.on));
}
