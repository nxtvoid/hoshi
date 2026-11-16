use crate::theme::ActiveTheme;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::rc::Rc;

/// The edge of the resized element a [`ResizeHandle`] sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeSide {
    Left,
    Right,
    Top,
    Bottom,
}

impl ResizeSide {
    fn vertical(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }

    /// Moving the pointer this way grows the element.
    fn direction(self) -> f32 {
        match self {
            Self::Right | Self::Bottom => 1.,
            Self::Left | Self::Top => -1.,
        }
    }
}

/// What a [`ResizeHandle`] drags.
pub struct ResizeDrag {
    id: ElementId,
}

type ChangeHandler = Rc<dyn Fn(f32, &mut Window, &mut App)>;

/// How far an arrow key moves a [`ResizeHandle`].
pub const KEYBOARD_STEP: f32 = 16.;

/// A draggable, focusable splitter on one edge of a panel. Double click restores the default
/// size; the arrows move it, Home and End take it to the smallest and largest size.
#[derive(IntoElement)]
pub struct ResizeHandle {
    id: ElementId,
    edge: ResizeSide,
    value: f32,
    min: f32,
    max: f32,
    default_value: f32,
    on_change: Option<ChangeHandler>,
}

/// The size a key asks for, `None` for a key the handle leaves alone.
pub fn keyed(value: f32, edge: ResizeSide, key: &str, range: (f32, f32)) -> Option<f32> {
    let (less, more) = if edge.vertical() {
        ("up", "down")
    } else {
        ("left", "right")
    };
    let next = match key {
        key if key == less => value - KEYBOARD_STEP * edge.direction(),
        key if key == more => value + KEYBOARD_STEP * edge.direction(),
        "home" => range.0,
        "end" => range.1,
        _ => return None,
    };
    Some(next.round().clamp(range.0, range.1))
}

impl ResizeHandle {
    /// `value` is the current size in pixels: the width on a side edge, the height on a top or
    /// bottom one.
    pub fn new(id: impl Into<ElementId>, edge: ResizeSide, value: f32) -> Self {
        Self {
            id: id.into(),
            edge,
            value,
            min: 0.,
            max: f32::MAX,
            default_value: value,
            on_change: None,
        }
    }

    pub fn range(mut self, min: f32, max: f32) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    /// Restored on double click.
    pub fn default_value(mut self, value: f32) -> Self {
        self.default_value = value;
        self
    }

    pub fn on_change(mut self, handler: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

/// The size a drag to `pointer` asks for, with the handle centered at `center`.
pub fn resized(value: f32, edge: ResizeSide, center: f32, pointer: f32, range: (f32, f32)) -> f32 {
    (value + (pointer - center) * edge.direction())
        .round()
        .clamp(range.0, range.1)
}

impl RenderOnce for ResizeHandle {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let hover = cx.hoshi().ring.opacity(0.4);
        let active = cx.hoshi().ring.opacity(0.6);
        let focus = window
            .use_keyed_state(self.id.clone(), cx, |_, cx| {
                cx.focus_handle().tab_stop(true)
            })
            .read(cx)
            .clone();
        let focus_visible = focus.is_focused(window) && window.last_input_was_keyboard();
        let Self {
            id,
            edge,
            value,
            min,
            max,
            default_value,
            on_change,
        } = self;
        let dragged = id.clone();
        let on_reset = on_change.clone();
        let on_key = on_change.clone();
        div()
            .id(id.clone())
            .track_focus(&focus)
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                gpui_kit::base::GlobalState::suppress_text_selection(cx);
            })
            .on_key_down(move |event, window, cx| {
                let Some(on_change) = &on_key else {
                    return;
                };
                let modifiers = event.keystroke.modifiers;
                if modifiers.control || modifiers.alt || modifiers.platform || modifiers.shift {
                    return;
                }
                if let Some(next) = keyed(value, edge, &event.keystroke.key, (min, max)) {
                    cx.stop_propagation();
                    if next != value {
                        on_change(next, window, cx);
                    }
                }
            })
            .when(focus_visible, move |handle| handle.bg(active))
            .absolute()
            .map(|handle| match edge {
                ResizeSide::Left => handle.left(px(-2.)).top_0().bottom_0().w_1(),
                ResizeSide::Right => handle.right(px(-2.)).top_0().bottom_0().w_1(),
                ResizeSide::Top => handle.top(px(-2.)).left_0().right_0().h_1(),
                ResizeSide::Bottom => handle.bottom(px(-2.)).left_0().right_0().h_1(),
            })
            .map(|handle| {
                if edge.vertical() {
                    handle.cursor_row_resize()
                } else {
                    handle.cursor_col_resize()
                }
            })
            .hover(move |style| style.bg(hover))
            .active(move |style| style.bg(active))
            .on_drag(ResizeDrag { id }, |_, _, _, cx| cx.new(|_| EmptyView))
            .on_drag_move::<ResizeDrag>(move |event, window, cx| {
                if event.drag(cx).id != dragged {
                    return;
                }
                let Some(on_change) = &on_change else {
                    return;
                };
                let (center, pointer) = if edge.vertical() {
                    (event.bounds.center().y, event.event.position.y)
                } else {
                    (event.bounds.center().x, event.event.position.x)
                };
                let next = resized(value, edge, center.into(), pointer.into(), (min, max));
                if next != value {
                    on_change(next, window, cx);
                }
            })
            .when_some(on_reset, |handle, on_change| {
                handle.on_click(move |event, window, cx| {
                    if event.click_count() == 2 {
                        on_change(default_value.clamp(min, max), window, cx);
                    }
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{ResizeSide, keyed, resized};

    #[test]
    fn keys_move_toward_the_side_that_grows_and_home_end_reach_the_ends() {
        let range = (320., 760.);
        assert_eq!(keyed(440., ResizeSide::Left, "left", range), Some(456.));
        assert_eq!(keyed(440., ResizeSide::Left, "right", range), Some(424.));
        assert_eq!(keyed(440., ResizeSide::Right, "right", range), Some(456.));
        assert_eq!(keyed(280., ResizeSide::Top, "up", (120., 800.)), Some(296.));
        assert_eq!(
            keyed(280., ResizeSide::Bottom, "up", (120., 800.)),
            Some(264.)
        );
        assert_eq!(keyed(440., ResizeSide::Left, "home", range), Some(320.));
        assert_eq!(keyed(440., ResizeSide::Left, "end", range), Some(760.));
        assert_eq!(keyed(755., ResizeSide::Left, "left", range), Some(760.));
        assert_eq!(keyed(440., ResizeSide::Left, "up", range), None);
        assert_eq!(keyed(440., ResizeSide::Left, "a", range), None);
    }

    #[test]
    fn a_left_edge_grows_as_the_pointer_moves_left() {
        assert_eq!(
            resized(440., ResizeSide::Left, 800., 760., (320., 760.)),
            480.
        );
        assert_eq!(
            resized(440., ResizeSide::Left, 800., 830., (320., 760.)),
            410.
        );
    }

    #[test]
    fn stays_in_range() {
        assert_eq!(
            resized(440., ResizeSide::Left, 800., 0., (320., 760.)),
            760.
        );
        assert_eq!(
            resized(440., ResizeSide::Bottom, 300., 0., (100., 600.)),
            140.
        );
        assert_eq!(
            resized(440., ResizeSide::Right, 300., 0., (320., 760.)),
            320.
        );
    }
}
