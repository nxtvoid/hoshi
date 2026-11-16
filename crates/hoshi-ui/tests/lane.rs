//! The lane centers, moves left for a floating card, and narrows when it can't move further.

use gpui_kit::component::theme::ThemeMode;
use gpui_kit::{
    AppContext as _, Bounds, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, Point, Render, Styled as _, TestAppContext, VisualTestContext, Window, WindowBounds,
    WindowOptions, div, px, size,
};
use hoshi_ui::{Lane, LaneRoom};

struct Harness;

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            Lane::new().child(
                div()
                    .debug_selector(|| "lane-content".into())
                    .w_full()
                    .h(px(20.)),
            ),
        )
    }
}

fn lane(cx: &mut TestAppContext, width: f32, room: LaneRoom) -> Bounds<Pixels> {
    cx.update(|cx| {
        gpui_kit::init(cx);
        hoshi_ui::theme::apply(ThemeMode::Dark, cx);
        LaneRoom::set(room, cx);
    });
    let window = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(width), px(200.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Harness))
            .expect("test window")
            .0
    });
    let mut cx = VisualTestContext::from_window(window, cx);
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.debug_bounds("lane-content").expect("the lane draws")
}

const CARD: LaneRoom = LaneRoom {
    right: px(324.),
    gutter: px(32.),
};

#[gpui_kit::test]
fn with_no_card_the_lane_is_centered(cx: &mut TestAppContext) {
    let bounds = lane(cx, 1200., LaneRoom::default());
    assert_eq!(bounds.size.width, px(768.));
    assert_eq!(bounds.origin.x, px(24. + (1152. - 768.) / 2.));
}

#[gpui_kit::test]
fn a_card_moves_the_lane_left_only_as_far_as_it_needs(cx: &mut TestAppContext) {
    let bounds = lane(cx, 1200., CARD);
    assert_eq!(bounds.size.width, px(768.));
    assert_eq!(bounds.origin.x, px(24. + 1152. - 768. - 324.));
}

#[gpui_kit::test]
fn a_card_with_no_room_left_narrows_the_lane(cx: &mut TestAppContext) {
    let bounds = lane(cx, 1000., CARD);
    assert_eq!(bounds.origin.x, px(24. + 32.));
    assert_eq!(bounds.size.width, px(952. - 32. - 324.));
}
