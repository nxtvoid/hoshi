use gpui_kit::base::{GlobalState, TextSelection};
use gpui_kit::*;

/// Whether the press being dispatched landed in a [`selection_area`].
#[derive(Default)]
struct PressInArea(bool);

impl Global for PressInArea {}

/// Decides where a text selection drag may start, and ends it on every left release.
///
/// A press starts one only on selectable text or inside a [`selection_area`]: buttons, rows and
/// popover triggers elsewhere never do, so moving the pointer after clicking one selects nothing.
/// gpui-base's selection layer hears the release last, so a clickable that stops it would leave
/// the drag following the pointer; the release is handled in the capture phase, before any of
/// them.
pub fn selection_release() -> impl IntoElement {
    canvas(
        |_, _, _| {},
        |_, _, window, _| {
            window.on_mouse_event(|event: &MouseDownEvent, phase, window, cx| {
                if !phase.bubble() || event.button != MouseButton::Left {
                    return;
                }
                let in_area = std::mem::take(&mut cx.default_global::<PressInArea>().0);
                if !in_area && !TextSelection::is_selectable_at(event.position, window, cx) {
                    GlobalState::suppress_text_selection(cx);
                }
            });
            window.on_mouse_event(|event: &MouseUpEvent, phase, window, cx| {
                if phase.capture() && event.button == MouseButton::Left {
                    cx.default_global::<PressInArea>().0 = false;
                    TextSelection::end(window, cx);
                }
            });
        },
    )
    .absolute()
    .size_0()
}

/// Marks the element it is a child of, which it covers, as text a drag may start a selection
/// from even where the press misses the glyphs, like the margins of a transcript. What is drawn
/// over it and blocks the mouse, such as a popover or a dialog, is not part of it.
pub fn selection_area() -> impl IntoElement {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        |_, hitbox, window, _| {
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if phase.capture()
                    && event.button == MouseButton::Left
                    && hitbox.is_hovered_at(event.position, window)
                {
                    cx.default_global::<PressInArea>().0 = true;
                }
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// For clickables drawn inside a [`selection_area`]: a left press on them starts no text
/// selection, as on a button.
pub trait PressSelectsNothing: InteractiveElement + Sized {
    fn press_selects_nothing(self) -> Self {
        self.on_mouse_down(MouseButton::Left, |_, _, cx| {
            GlobalState::suppress_text_selection(cx)
        })
    }
}

impl<E: InteractiveElement> PressSelectsNothing for E {}
