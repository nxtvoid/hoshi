//! A popup's place: the shared [`place`] picks the side and alignment for its natural size, then
//! the popup is cut down to the room on that side, as Base UI's `--available-height`, and the
//! pointer's way from the trigger to it is kept safe for popovers opened by hover.

use crate::components::placement::{Placement, Side, available, origin_on, place};
use gpui_kit::{Bounds, Pixels, Point, Size, size};

/// Where a popup ended up, cut down to the room it has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Fitted {
    pub bounds: Bounds<Pixels>,
    pub side: Side,
}

/// Places a popup of `natural` size next to `anchor`, inside `boundary`.
pub(super) fn fit(
    anchor: Bounds<Pixels>,
    natural: Size<Pixels>,
    boundary: Bounds<Pixels>,
    placement: Placement,
) -> Fitted {
    let inside = Bounds::new(anchor.origin - boundary.origin, anchor.size);
    let viewport = boundary.size;
    let placed = place(inside, natural, viewport, placement);
    let room = available(inside, placed.side, viewport, placement);
    let fitted = size(
        natural.width.min(room.width),
        natural.height.min(room.height),
    );
    let origin = origin_on(
        inside,
        fitted,
        viewport,
        placed.side,
        placed.align,
        placement,
    );
    Fitted {
        bounds: Bounds::new(origin + boundary.origin, fitted),
        side: placed.side,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Rect {
    fn from_bounds(bounds: Bounds<Pixels>) -> Self {
        Self {
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
            w: f32::from(bounds.size.width),
            h: f32::from(bounds.size.height),
        }
    }

    fn right(&self) -> f32 {
        self.x + self.w
    }

    fn bottom(&self) -> f32 {
        self.y + self.h
    }
}
/// Whether `pointer` is where it may pass on its way from the trigger to the popup: over either,
/// in the gap between them, or inside the triangle from where it left the trigger to the
/// popup's near edge.
pub(super) fn in_safe_area(
    trigger: Bounds<Pixels>,
    popup: Bounds<Pixels>,
    side: Side,
    exit: Option<Point<Pixels>>,
    pointer: Point<Pixels>,
) -> bool {
    if trigger.contains(&pointer) || popup.contains(&pointer) {
        return true;
    }
    let (t, p) = (Rect::from_bounds(trigger), Rect::from_bounds(popup));
    let (px_, py) = (f32::from(pointer.x), f32::from(pointer.y));
    let gap = match side {
        Side::Bottom => Rect {
            x: t.x.min(p.x),
            y: t.bottom(),
            w: t.right().max(p.right()) - t.x.min(p.x),
            h: p.y - t.bottom(),
        },
        Side::Top => Rect {
            x: t.x.min(p.x),
            y: p.bottom(),
            w: t.right().max(p.right()) - t.x.min(p.x),
            h: t.y - p.bottom(),
        },
        Side::Right => Rect {
            x: t.right(),
            y: t.y.min(p.y),
            w: p.x - t.right(),
            h: t.bottom().max(p.bottom()) - t.y.min(p.y),
        },
        Side::Left => Rect {
            x: p.right(),
            y: t.y.min(p.y),
            w: t.x - p.right(),
            h: t.bottom().max(p.bottom()) - t.y.min(p.y),
        },
    };
    if gap.w > 0.
        && gap.h > 0.
        && px_ >= gap.x
        && px_ <= gap.right()
        && py >= gap.y
        && py <= gap.bottom()
    {
        return true;
    }
    let Some(exit) = exit else {
        return false;
    };
    let apex = (f32::from(exit.x), f32::from(exit.y));
    let (a, b) = match side {
        Side::Bottom => ((p.x, p.y), (p.right(), p.y)),
        Side::Top => ((p.x, p.bottom()), (p.right(), p.bottom())),
        Side::Right => ((p.x, p.y), (p.x, p.bottom())),
        Side::Left => ((p.right(), p.y), (p.right(), p.bottom())),
    };
    in_triangle((px_, py), apex, a, b)
}

fn in_triangle(p: (f32, f32), a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let cross = |o: (f32, f32), u: (f32, f32), v: (f32, f32)| {
        (u.0 - o.0) * (v.1 - o.1) - (u.1 - o.1) * (v.0 - o.0)
    };
    let (d1, d2, d3) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
    let negative = d1 < 0. || d2 < 0. || d3 < 0.;
    let positive = d1 > 0. || d2 > 0. || d3 > 0.;
    !(negative && positive)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::placement::Align;
    use gpui_kit::{point, px};

    fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    fn window() -> Bounds<Pixels> {
        bounds(0., 0., 800., 600.)
    }

    fn menu(side: Side, align: Align) -> Placement {
        Placement {
            side,
            align,
            side_offset: px(4.),
            fallback_axis: false,
            ..Placement::default()
        }
    }

    #[test]
    fn a_menu_too_tall_for_either_side_keeps_the_roomier_one_and_shrinks_to_fit() {
        let fitted = fit(
            bounds(100., 200., 80., 24.),
            size(px(160.), px(500.)),
            window(),
            menu(Side::Top, Align::Start),
        );
        assert_eq!(fitted.side, Side::Bottom);
        assert_eq!(fitted.bounds, bounds(100., 228., 160., 600. - 5. - 228.));
    }

    #[test]
    fn a_shrunk_popup_on_top_still_ends_at_the_trigger() {
        let fitted = fit(
            bounds(100., 400., 80., 24.),
            size(px(160.), px(500.)),
            window(),
            menu(Side::Top, Align::Start),
        );
        assert_eq!(fitted.side, Side::Top);
        assert_eq!(fitted.bounds.top(), px(5.));
        assert_eq!(fitted.bounds.bottom(), px(396.));
    }

    #[test]
    fn a_popover_may_move_to_the_other_axis_but_a_menu_may_not() {
        let anchor = bounds(100., 280., 40., 40.);
        let popup = size(px(200.), px(300.));
        let popover = Placement {
            fallback_axis: true,
            ..menu(Side::Bottom, Align::Center)
        };
        let fitted = fit(anchor, popup, window(), popover);
        assert_eq!(fitted.side, Side::Right);
        assert_eq!(fitted.bounds.left(), px(144.));
        assert_eq!(fitted.bounds.size.height, px(300.));

        let fitted = fit(anchor, popup, window(), menu(Side::Bottom, Align::Center));
        assert!(fitted.side.is_vertical());
        assert!(fitted.bounds.size.height < px(300.));
    }

    #[test]
    fn a_popup_wider_than_the_window_takes_its_width() {
        let fitted = fit(
            bounds(100., 100., 40., 24.),
            size(px(1000.), px(40.)),
            window(),
            menu(Side::Bottom, Align::Start),
        );
        assert_eq!(fitted.bounds.left(), px(5.));
        assert_eq!(fitted.bounds.size.width, px(790.));
    }

    #[test]
    fn the_boundary_can_leave_out_the_window_frame() {
        let fitted = fit(
            bounds(760., 100., 30., 24.),
            size(px(100.), px(40.)),
            bounds(10., 10., 780., 580.),
            menu(Side::Bottom, Align::Start),
        );
        assert_eq!(fitted.bounds.right(), px(785.));
        assert_eq!(fitted.bounds.top(), px(128.));
    }

    #[test]
    fn the_way_from_the_trigger_to_the_popup_is_safe() {
        let trigger = bounds(100., 100., 40., 20.);
        let popup = bounds(60., 130., 200., 100.);
        let exit = Some(point(px(140.), px(110.)));
        let safe =
            |x: f32, y: f32| in_safe_area(trigger, popup, Side::Bottom, exit, point(px(x), px(y)));
        assert!(safe(120., 110.), "over the trigger");
        assert!(safe(200., 125.), "in the gap");
        assert!(safe(240., 150.), "over the popup");
        assert!(safe(150., 118.), "on the way down");
        assert!(!safe(200., 112.), "beside the way");
        assert!(!safe(300., 125.), "past the popup");
        assert!(!safe(120., 90.), "away from the popup");
        assert!(!in_safe_area(
            trigger,
            popup,
            Side::Bottom,
            None,
            point(px(30.), px(129.))
        ));
    }
}
