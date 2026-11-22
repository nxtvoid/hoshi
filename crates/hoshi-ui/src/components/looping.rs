use gpui_kit::*;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// How often the animations that loop, such as spinners and shimmers, are drawn. The redraw
/// waits for the display's next frame after the tick, so this draws them about 30 times a second
/// on a 60 or 120 Hz display; ticking at 30 would draw them at 20 on a 60 Hz one.
pub const LOOP_FPS: f32 = 40.;

/// An animation that repeats every `duration`, drawn on the [`LOOP_FPS`] clock.
pub fn looping(duration: Duration) -> Loop {
    Loop {
        duration,
        easing: Rc::new(|delta| delta),
    }
}

#[derive(Clone)]
pub struct Loop {
    duration: Duration,
    easing: Rc<dyn Fn(f32) -> f32>,
}

impl Loop {
    pub fn with_easing(mut self, easing: impl Fn(f32) -> f32 + 'static) -> Self {
        self.easing = Rc::new(easing);
        self
    }

    /// Where the loop is now, eased: every loop of the same length is at the same point, as
    /// they all count from the clock's start.
    fn delta(&self, elapsed: Duration) -> f32 {
        let nanos = self.duration.as_nanos().max(1);
        let delta = (elapsed.as_nanos() % nanos) as f32 / nanos as f32;
        (self.easing)(delta)
    }
}

pub trait LoopExt: IntoElement + Sized + 'static {
    /// Draws this through `animator` with where the loop is now. All loops on screen redraw on
    /// one shared tick, so however many show, the window draws them at most [`LOOP_FPS`] times
    /// a second; with reduced motion they hold still at the start.
    fn with_loop(
        self,
        looped: Loop,
        animator: impl Fn(Self, f32) -> Self + 'static,
    ) -> Looping<Self> {
        Looping {
            element: Some(self),
            looped,
            animator: Box::new(animator),
        }
    }
}

impl<E: IntoElement + 'static> LoopExt for E {}

pub struct Looping<E> {
    element: Option<E>,
    looped: Loop,
    animator: Box<dyn Fn(E, f32) -> E>,
}

impl<E: IntoElement + 'static> IntoElement for Looping<E> {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl<E: IntoElement + 'static> Element for Looping<E> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, AnyElement) {
        let elapsed = if cx.reduce_motion() {
            Duration::ZERO
        } else {
            LoopClock::wait_for_tick(window.current_view(), cx)
        };
        let element = self.element.take().expect("drawn once");
        let mut element = (self.animator)(element, self.looped.delta(elapsed)).into_any_element();
        (element.request_layout(window, cx), element)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        element: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) {
        element.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        element: &mut AnyElement,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        element.paint(window, cx);
    }
}

/// The one tick every loop on screen waits for: the views that drew a loop are told to draw
/// again on the next multiple of the interval since the clock started.
#[derive(Default)]
struct LoopClock {
    start: Option<Instant>,
    views: HashSet<EntityId>,
    ticking: bool,
}

impl Global for LoopClock {}

impl LoopClock {
    /// Asks for `view` to be drawn on the next tick; returns the time since the clock started.
    fn wait_for_tick(view: EntityId, cx: &mut App) -> Duration {
        let now = cx.background_executor().now();
        let clock = cx.default_global::<LoopClock>();
        let elapsed = now - *clock.start.get_or_insert(now);
        clock.views.insert(view);
        if !clock.ticking {
            clock.ticking = true;
            let interval = Duration::from_secs_f32(1. / LOOP_FPS);
            let wait =
                interval - Duration::from_nanos((elapsed.as_nanos() % interval.as_nanos()) as u64);
            cx.spawn(async move |cx| {
                cx.background_executor().timer(wait).await;
                cx.update(|cx| {
                    let clock = cx.default_global::<LoopClock>();
                    clock.ticking = false;
                    for view in std::mem::take(&mut clock.views) {
                        cx.notify(view);
                    }
                });
            })
            .detach();
        }
        elapsed
    }
}

#[cfg(test)]
mod tests {
    use super::{LoopExt as _, looping};
    use gpui_kit::prelude::FluentBuilder as _;
    use gpui_kit::{
        AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _, Point, Render,
        Styled as _, TestAppContext, VisualTestContext, Window, WindowBounds, WindowOptions, div,
        px, size,
    };
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    struct Board {
        first: bool,
        second: bool,
        draws: Rc<Cell<usize>>,
    }

    impl Render for Board {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            self.draws.set(self.draws.get() + 1);
            let pulse = |millis| {
                div()
                    .size_4()
                    .with_loop(looping(Duration::from_millis(millis)), |dot, delta| {
                        dot.opacity(delta)
                    })
            };
            div()
                .size_full()
                .when(self.first, |board| board.child(pulse(1000)))
                .when(self.second, |board| board.child(pulse(1600)))
        }
    }

    fn open(cx: &mut TestAppContext) -> (VisualTestContext, Entity<Board>, Rc<Cell<usize>>) {
        cx.update(gpui_kit::init);
        let draws = Rc::new(Cell::new(0));
        let shared = draws.clone();
        let (window, board) = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(200.), px(200.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |_, cx| {
                cx.new(|_| Board {
                    first: true,
                    second: false,
                    draws: shared,
                })
            })
            .expect("test window")
        });
        let mut cx = VisualTestContext::from_window(window, cx);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        (cx, board, draws)
    }

    fn run_for(cx: &mut VisualTestContext, millis: u64) {
        for _ in 0..millis {
            cx.executor().advance_clock(Duration::from_millis(1));
            cx.run_until_parked();
        }
    }

    #[gpui_kit::test]
    fn loops_that_start_apart_redraw_the_window_on_one_tick(cx: &mut TestAppContext) {
        let (mut cx, board, draws) = open(cx);
        run_for(&mut cx, 13);
        board.update(&mut cx, |board, cx| {
            board.second = true;
            cx.notify();
        });
        run_for(&mut cx, 50);
        draws.set(0);
        run_for(&mut cx, 1000);
        let drawn = draws.get();
        assert!(
            (38..=42).contains(&drawn),
            "drawn {drawn} times in a second"
        );
    }

    #[gpui_kit::test]
    fn the_ticks_stop_once_no_loop_is_drawn(cx: &mut TestAppContext) {
        let (mut cx, board, draws) = open(cx);
        run_for(&mut cx, 100);
        board.update(&mut cx, |board, cx| {
            board.first = false;
            cx.notify();
        });
        run_for(&mut cx, 50);
        draws.set(0);
        run_for(&mut cx, 1000);
        assert_eq!(draws.get(), 0);
    }
}
