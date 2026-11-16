//! The element that draws an open popup: it lays the popup out against the trigger's bounds of
//! this very frame, blocks what is under it, and watches presses and the pointer around it.

use super::PopoverChangeReason;
use super::position::{fit, in_safe_area};
use super::state::{Method, OpenPopovers, PopoverState};
use crate::components::placement::{Placement, Side};
use gpui_kit::*;

/// How far the popup travels towards its place while it fades in.
const ENTER_TRAVEL: f32 = 2.;

pub(super) struct PopupHost {
    state: Entity<PopoverState>,
    content: Option<AnyElement>,
    placement: Placement,
    modal: bool,
    progress: f32,
}

impl PopupHost {
    pub fn new(
        state: Entity<PopoverState>,
        content: AnyElement,
        placement: Placement,
        modal: bool,
        progress: f32,
    ) -> Self {
        Self {
            state,
            content: Some(content),
            placement,
            modal,
            progress,
        }
    }
}

pub(super) struct Placed {
    popup: Bounds<Pixels>,
}

impl IntoElement for PopupHost {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

/// The part of the window a popup may use: the viewport, less the frame a window with
/// client-side decorations draws around it on its untiled edges.
fn boundary(window: &Window) -> Bounds<Pixels> {
    let viewport = window.viewport_size();
    let inset = window.client_inset().unwrap_or_default();
    let (top, right, bottom, left) = match window.window_decorations() {
        Decorations::Server => (px(0.), px(0.), px(0.), px(0.)),
        Decorations::Client { tiling } => {
            let edge = |tiled: bool| if tiled { px(0.) } else { inset };
            (
                edge(tiling.top),
                edge(tiling.right),
                edge(tiling.bottom),
                edge(tiling.left),
            )
        }
    };
    Bounds::from_corners(
        point(left, top),
        point(viewport.width - right, viewport.height - bottom),
    )
}

/// The window around the trigger, in four strips: what a modal popup blocks.
fn backdrop(window: Bounds<Pixels>, trigger: Bounds<Pixels>) -> [Bounds<Pixels>; 4] {
    let (left, top) = (window.left(), window.top());
    let (right, bottom) = (window.right(), window.bottom());
    [
        Bounds::from_corners(point(left, top), point(right, trigger.top())),
        Bounds::from_corners(point(left, trigger.bottom()), point(right, bottom)),
        Bounds::from_corners(
            point(left, trigger.top()),
            point(trigger.left(), trigger.bottom()),
        ),
        Bounds::from_corners(
            point(trigger.right(), trigger.top()),
            point(right, trigger.bottom()),
        ),
    ]
}

impl Element for PopupHost {
    type RequestLayoutState = ();
    type PrepaintState = Option<Placed>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let style = Style {
            position: Position::Absolute,
            ..Style::default()
        };
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Placed> {
        let anchor = self.state.read(cx).anchor.get()?;
        if !anchor.visible {
            return None;
        }
        let content = self.content.as_mut()?;
        let id = self.state.entity_id();
        let boundary = boundary(window);

        OpenPopovers::begin_render(cx, id);
        let natural = content.layout_as_root(
            size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
            window,
            cx,
        );
        OpenPopovers::end_render(cx);
        let placed = fit(anchor.bounds, natural, boundary, self.placement);
        content.layout_as_root(
            size(
                AvailableSpace::Definite(placed.bounds.size.width),
                AvailableSpace::Definite(placed.bounds.size.height),
            ),
            window,
            cx,
        );

        let travel = px((1. - self.progress) * ENTER_TRAVEL);
        let slide = match placed.side {
            Side::Bottom => point(px(0.), -travel),
            Side::Top => point(px(0.), travel),
            Side::Right => point(-travel, px(0.)),
            Side::Left => point(travel, px(0.)),
        };
        let origin = placed.bounds.origin + slide;
        let origin = match placed.side {
            Side::Top => point(origin.x.round(), origin.y.floor()),
            Side::Bottom => point(origin.x.round(), origin.y.ceil()),
            Side::Left => point(origin.x.floor(), origin.y.round()),
            Side::Right => point(origin.x.ceil(), origin.y.round()),
        };

        let (method, modal) = {
            let state = self.state.read(cx);
            (state.method, self.modal)
        };
        if modal && method != Method::Hover {
            for strip in backdrop(
                Bounds::new(point(px(0.), px(0.)), window.viewport_size()),
                anchor.bounds,
            ) {
                if !strip.is_empty() {
                    window.insert_hitbox(strip, HitboxBehavior::BlockMouse);
                }
            }
        }
        window.insert_hitbox(placed.bounds, HitboxBehavior::BlockMouse);
        content.prepaint_at(origin, window, cx);

        self.state.update(cx, |state, _| state.side = placed.side);
        OpenPopovers::place(cx, id, anchor.bounds, placed.bounds);
        Some(Placed {
            popup: placed.bounds,
        })
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        placed: &mut Option<Placed>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(placed) = placed.as_ref() else {
            return;
        };
        let id = self.state.entity_id();
        let popup = placed.popup;

        let state = self.state.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, _, cx| {
            if phase != DispatchPhase::Capture {
                return;
            }
            let outside = !OpenPopovers::is_inside(cx, id, event.position);
            state.update(cx, |state, _| {
                state.pressing = true;
                state.press_outside = outside;
                state.menu.drag_open = false;
            });
        });

        let state = self.state.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture {
                return;
            }
            let outside = !OpenPopovers::is_inside(cx, id, event.position);
            state.update(cx, |state, cx| {
                let pressed_outside = std::mem::take(&mut state.press_outside);
                state.pressing = false;
                if pressed_outside && outside && state.open {
                    state.close(PopoverChangeReason::OutsidePress, window, cx);
                }
            });
        });

        let state = self.state.clone();
        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble {
                state.update(cx, |state, _| state.menu.drag_open = false);
            }
        });

        if self.state.read(cx).method == Method::Hover {
            let state = self.state.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble {
                    return;
                }
                let (trigger, side, exit) = {
                    let state = state.read(cx);
                    let Some(anchor) = state.anchor.get() else {
                        return;
                    };
                    (anchor.bounds, state.side, state.exit_point)
                };
                let safe = in_safe_area(trigger, popup, side, exit, event.position)
                    || OpenPopovers::is_inside(cx, id, event.position);
                let over_popup = popup.contains(&event.position);
                state.update(cx, |state, cx| {
                    state.track_pointer(safe, over_popup, window, cx)
                });
            });
        }

        if let Some(content) = self.content.as_mut() {
            content.paint(window, cx);
        }
    }
}
