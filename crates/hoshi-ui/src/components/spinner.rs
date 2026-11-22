use super::looping::{LoopExt as _, looping};
use crate::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::*;
use std::time::Duration;

/// The loading circle, turning once a second.
pub fn spinner(size: Pixels, color: Hsla) -> AnyElement {
    Icon::new(IconName::LoaderCircle)
        .size(size)
        .flex_none()
        .text_color(color)
        .with_loop(looping(Duration::from_secs(1)), |icon, delta| {
            icon.transform(Transformation::rotate(percentage(delta)))
        })
        .into_any_element()
}
