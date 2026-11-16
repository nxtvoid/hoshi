use crate::theme::ActiveTheme;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::cell::Cell;
use std::rc::Rc;

/// Hue in degrees, saturation and value from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hsv {
    pub h: f32,
    pub s: f32,
    pub v: f32,
}

impl Hsv {
    pub fn from_rgb(color: Rgba) -> Self {
        let (r, g, b) = (color.r, color.g, color.b);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        let h = if delta == 0. {
            0.
        } else if max == r {
            60. * ((g - b) / delta).rem_euclid(6.)
        } else if max == g {
            60. * ((b - r) / delta + 2.)
        } else {
            60. * ((r - g) / delta + 4.)
        };
        let s = if max == 0. { 0. } else { delta / max };
        Self { h, s, v: max }
    }

    pub fn to_rgb(self) -> Rgba {
        let c = self.v * self.s;
        let h = (self.h.rem_euclid(360.)) / 60.;
        let x = c * (1. - (h.rem_euclid(2.) - 1.).abs());
        let (r, g, b) = match h as u32 {
            0 => (c, x, 0.),
            1 => (x, c, 0.),
            2 => (0., c, x),
            3 => (0., x, c),
            4 => (x, 0., c),
            _ => (c, 0., x),
        };
        let m = self.v - c;
        Rgba {
            r: r + m,
            g: g + m,
            b: b + m,
            a: 1.,
        }
    }
}

/// `#rrggbb` for an opaque color.
pub fn rgb_to_hex(color: Rgba) -> String {
    let channel = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        channel(color.r),
        channel(color.g),
        channel(color.b)
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Target {
    Area,
    Hue,
}

pub enum ColorPickerEvent {
    /// The color changed while dragging or with the keyboard.
    Change(Rgba),
}

/// A saturation and brightness area over a hue bar, like the web's color picker.
pub struct ColorPicker {
    hsv: Hsv,
    area: Rc<Cell<Bounds<Pixels>>>,
    hue: Rc<Cell<Bounds<Pixels>>>,
    drag: Rc<Cell<Option<Target>>>,
}

impl EventEmitter<ColorPickerEvent> for ColorPicker {}

impl ColorPicker {
    pub fn new(color: Rgba) -> Self {
        Self {
            hsv: Hsv::from_rgb(color),
            area: Rc::default(),
            hue: Rc::default(),
            drag: Rc::default(),
        }
    }

    pub fn color(&self) -> Rgba {
        self.hsv.to_rgb()
    }

    /// Shows `color`, keeping the hue when it is gray and the hue can't be told.
    pub fn set_color(&mut self, color: Rgba, cx: &mut Context<Self>) {
        if rgb_to_hex(self.color()) == rgb_to_hex(color) {
            return;
        }
        let next = Hsv::from_rgb(color);
        self.hsv = if next.s == 0. || next.v == 0. {
            Hsv {
                h: self.hsv.h,
                ..next
            }
        } else {
            next
        };
        cx.notify();
    }

    fn move_to(&mut self, target: Target, position: Point<Pixels>, cx: &mut Context<Self>) {
        let fraction = |bounds: Bounds<Pixels>| {
            let x = ((position.x - bounds.left()) / bounds.size.width).clamp(0., 1.);
            let y = ((position.y - bounds.top()) / bounds.size.height).clamp(0., 1.);
            (x, y)
        };
        match target {
            Target::Area => {
                let (x, y) = fraction(self.area.get());
                self.hsv.s = x;
                self.hsv.v = 1. - y;
            }
            Target::Hue => {
                let (x, _) = fraction(self.hue.get());
                self.hsv.h = (x * 360.).min(359.9);
            }
        }
        cx.emit(ColorPickerEvent::Change(self.color()));
        cx.notify();
    }

    fn start(&mut self, target: Target, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.drag.set(Some(target));
        self.move_to(target, position, cx);
    }
}

fn pure_hue(h: f32) -> Hsla {
    Hsv { h, s: 1., v: 1. }.to_rgb().into()
}

fn measure(cell: Rc<Cell<Bounds<Pixels>>>) -> impl IntoElement {
    canvas(move |bounds, _, _| cell.set(bounds), |_, _, _, _| {})
        .absolute()
        .size_full()
}

impl Render for ColorPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.hoshi();
        let ring = colors.background;
        let hsv = self.hsv;
        let current: Hsla = hsv.to_rgb().into();
        let entity = cx.entity().downgrade();
        let drag = self.drag.clone();
        let thumb = |color: Hsla| {
            div()
                .absolute()
                .size(px(14.))
                .rounded_full()
                .border_2()
                .border_color(ring)
                .bg(color)
                .shadow_sm()
        };
        let stops = [0., 60., 120., 180., 240., 300., 360.];

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .id("color-area")
                    .relative()
                    .h(px(144.))
                    .rounded_md()
                    .cursor_crosshair()
                    .bg(pure_hue(hsv.h))
                    .child(measure(self.area.clone()))
                    .child(
                        div()
                            .absolute()
                            .size_full()
                            .rounded_md()
                            .bg(linear_gradient(
                                90.,
                                linear_color_stop(white(), 0.),
                                linear_color_stop(white().opacity(0.), 1.),
                            )),
                    )
                    .child(
                        div()
                            .absolute()
                            .size_full()
                            .rounded_md()
                            .bg(linear_gradient(
                                0.,
                                linear_color_stop(black(), 0.),
                                linear_color_stop(black().opacity(0.), 1.),
                            )),
                    )
                    .child(
                        thumb(current)
                            .left(relative(hsv.s))
                            .top(relative(1. - hsv.v))
                            .ml(px(-7.))
                            .mt(px(-7.)),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            this.start(Target::Area, event.position, cx)
                        }),
                    ),
            )
            .child(
                div()
                    .id("color-hue")
                    .relative()
                    .h_3()
                    .rounded_full()
                    .cursor_pointer()
                    .child(measure(self.hue.clone()))
                    .child(
                        div()
                            .absolute()
                            .size_full()
                            .flex()
                            .rounded_full()
                            .overflow_hidden()
                            .children(stops.windows(2).enumerate().map(|(index, pair)| {
                                div()
                                    .flex_1()
                                    .h_full()
                                    .when(index == 0, |this| this.rounded_l_full())
                                    .when(index == 5, |this| this.rounded_r_full())
                                    .bg(linear_gradient(
                                        90.,
                                        linear_color_stop(pure_hue(pair[0]), 0.),
                                        linear_color_stop(pure_hue(pair[1]), 1.),
                                    ))
                            })),
                    )
                    .child(
                        thumb(pure_hue(hsv.h))
                            .left(relative(hsv.h / 360.))
                            .top(px(-1.))
                            .ml(px(-7.)),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            this.start(Target::Hue, event.position, cx)
                        }),
                    ),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        let moving = (entity.clone(), drag.clone());
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if phase != DispatchPhase::Bubble || !event.dragging() {
                                return;
                            }
                            if let Some(target) = moving.1.get() {
                                moving
                                    .0
                                    .update(cx, |this, cx| this.move_to(target, event.position, cx))
                                    .ok();
                            }
                        });
                        let ending = drag.clone();
                        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, _| {
                            if phase == DispatchPhase::Bubble {
                                ending.set(None);
                            }
                        });
                    },
                )
                .absolute()
                .size_0(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{Hsv, rgb_to_hex};
    use gpui_kit::Rgba;

    fn rgb(r: f32, g: f32, b: f32) -> Rgba {
        Rgba { r, g, b, a: 1. }
    }

    #[test]
    fn hsv_round_trips_through_rgb() {
        for color in [rgb(1., 0., 0.), rgb(0.2, 0.6, 0.4), rgb(0.31, 0.27, 0.9)] {
            let back = Hsv::from_rgb(color).to_rgb();
            assert_eq!(rgb_to_hex(back), rgb_to_hex(color));
        }
    }

    #[test]
    fn pure_colors_have_their_hue() {
        assert_eq!(Hsv::from_rgb(rgb(0., 1., 0.)).h, 120.);
        assert_eq!(Hsv::from_rgb(rgb(0., 0., 1.)).h, 240.);
        assert_eq!(rgb_to_hex(rgb(1., 0.5, 0.)), "#ff8000");
    }
}
