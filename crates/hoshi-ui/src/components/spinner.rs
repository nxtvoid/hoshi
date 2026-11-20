use crate::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::*;
use std::time::Duration;

/// The rate of the timer that redraws animations that loop, such as spinners and shimmers,
/// rather than every frame of the display. The redraw waits for the display's next frame after
/// the timer, so this draws them about 30 times a second on a 60 or 120 Hz display; a timer at
/// 30 would draw them at 20 on a 60 Hz one.
pub const LOOP_FPS: f32 = 40.;

/// An animation that loops, on a [`LOOP_FPS`] timer.
pub fn looping(duration: Duration) -> Animation {
    Animation::new(duration).repeat().with_max_fps(LOOP_FPS)
}

/// The loading circle, turning once a second.
pub fn spinner(size: Pixels, color: Hsla) -> AnyElement {
    Icon::new(IconName::LoaderCircle)
        .size(size)
        .flex_none()
        .text_color(color)
        .with_animation("spinner", looping(Duration::from_secs(1)), |icon, delta| {
            icon.transform(Transformation::rotate(percentage(delta)))
        })
        .into_any_element()
}
