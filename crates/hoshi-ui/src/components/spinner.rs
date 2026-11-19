use crate::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::*;
use std::time::Duration;

/// The frame rate of animations that loop, such as spinners and shimmers: they redraw what holds
/// them on a timer at this rate rather than on every frame of the display.
pub const LOOP_FPS: f32 = 30.;

/// An animation that loops, at [`LOOP_FPS`].
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
