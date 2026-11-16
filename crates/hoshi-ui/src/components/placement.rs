//! Where a floating surface goes next to its anchor: the side and alignment asked for, flipped
//! when it does not fit and shifted to stay inside the window, as Floating UI does for Base UI.

use gpui_kit::{Bounds, Pixels, Point, Size, point, px};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Side {
    #[default]
    Top,
    Right,
    Bottom,
    Left,
}

impl Side {
    pub fn opposite(self) -> Self {
        match self {
            Self::Top => Self::Bottom,
            Self::Right => Self::Left,
            Self::Bottom => Self::Top,
            Self::Left => Self::Right,
        }
    }

    /// Above or below the anchor, so aligned along the x axis.
    pub fn is_vertical(self) -> bool {
        matches!(self, Self::Top | Self::Bottom)
    }

    fn perpendicular(self) -> [Self; 2] {
        if self.is_vertical() {
            [Self::Right, Self::Left]
        } else {
            [Self::Bottom, Self::Top]
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    Start,
    #[default]
    Center,
    End,
}

impl Align {
    pub fn flipped(self) -> Self {
        match self {
            Self::Start => Self::End,
            Self::Center => Self::Center,
            Self::End => Self::Start,
        }
    }
}

/// What a surface asks for. Offsets and padding are Base UI's positioner props.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub side: Side,
    pub align: Align,
    pub side_offset: Pixels,
    pub align_offset: Pixels,
    /// The room kept between the surface and the window edges.
    pub collision_padding: Pixels,
    /// The arrow's width along the edge it points from, or zero without an arrow.
    pub arrow: Pixels,
    /// The room kept between the arrow and the surface's corners.
    pub arrow_padding: Pixels,
    /// Whether the surface may move to the other axis when neither side of its own fits.
    pub fallback_axis: bool,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            side: Side::Top,
            align: Align::Center,
            side_offset: px(0.),
            align_offset: px(0.),
            collision_padding: px(5.),
            arrow: px(0.),
            arrow_padding: px(5.),
            fallback_axis: true,
        }
    }
}

/// Where the surface went.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    pub origin: Point<Pixels>,
    pub side: Side,
    pub align: Align,
    /// The arrow's center along the surface's edge, from the surface's left or top.
    pub arrow_center: Pixels,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

#[derive(Clone, Copy)]
struct Edges {
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
}

impl Edges {
    fn get(&self, side: Side) -> f32 {
        match side {
            Side::Top => self.top,
            Side::Right => self.right,
            Side::Bottom => self.bottom,
            Side::Left => self.left,
        }
    }
}

struct Tried {
    side: Side,
    align: Align,
    main: f32,
    cross: [f32; 2],
}

/// Places a surface of `size` next to `anchor`, both in window coordinates.
pub fn place(
    anchor: Bounds<Pixels>,
    size: Size<Pixels>,
    viewport: Size<Pixels>,
    placement: Placement,
) -> Placed {
    let anchor = Rect {
        x: anchor.origin.x.into(),
        y: anchor.origin.y.into(),
        w: anchor.size.width.into(),
        h: anchor.size.height.into(),
    };
    let (w, h) = (f32::from(size.width), f32::from(size.height));
    let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
    let padding = f32::from(placement.collision_padding);
    let bias = |side: Side| {
        if side == placement.side.opposite() {
            2.
        } else {
            1.
        }
    };
    let flip_padding = Edges {
        top: padding + bias(Side::Top),
        right: padding + bias(Side::Right),
        bottom: padding + bias(Side::Bottom),
        left: padding + bias(Side::Left),
    };

    let position = |side: Side, align: Align| {
        let rect = raw(anchor, w, h, side, align, placement);
        if align == Align::Center {
            shift(rect, anchor, side, vw, vh, placement)
        } else {
            rect
        }
    };

    let candidates = candidates(placement.side, placement.align, placement.fallback_axis);
    let mut tried: Vec<Tried> = Vec::with_capacity(candidates.len());
    let mut chosen = (placement.side, placement.align);
    for (index, &(side, align)) in candidates.iter().enumerate() {
        let overflow = overflow(position(side, align), vw, vh, flip_padding);
        let cross = if side.is_vertical() {
            [overflow.left, overflow.right]
        } else {
            [overflow.top, overflow.bottom]
        };
        let main = overflow.get(side);
        tried.push(Tried {
            side,
            align,
            main,
            cross,
        });
        if main <= 0. && cross.iter().all(|overflow| *overflow <= 0.) {
            chosen = (side, align);
            break;
        }
        let same_axis = |side: Side| side.is_vertical() == placement.side.is_vertical();
        if let Some(&(next, _)) = candidates.get(index + 1)
            && (same_axis(next)
                || tried
                    .iter()
                    .all(|tried| !same_axis(tried.side) || tried.main > 0.))
        {
            continue;
        }
        chosen = settle(&tried, placement.side).unwrap_or(chosen);
        break;
    }

    let (side, align) = chosen;
    let rect = shift(
        raw(anchor, w, h, side, align, placement),
        anchor,
        side,
        vw,
        vh,
        placement,
    );
    let (anchor_center, start, length) = if side.is_vertical() {
        (anchor.x + anchor.w / 2., rect.x, rect.w)
    } else {
        (anchor.y + anchor.h / 2., rect.y, rect.h)
    };
    let half_arrow = f32::from(placement.arrow) / 2.;
    let edge = f32::from(placement.arrow_padding) + half_arrow;
    let arrow_center = (anchor_center - start).min(length - edge).max(edge);
    Placed {
        origin: point(px(rect.x), px(rect.y)),
        side,
        align,
        arrow_center: px(arrow_center),
    }
}

/// The room beside the anchor on `side`, which bounds a surface's size there: up to the window
/// edge along the side, the window less its padding across it.
pub fn available(
    anchor: Bounds<Pixels>,
    side: Side,
    viewport: Size<Pixels>,
    placement: Placement,
) -> Size<Pixels> {
    let anchor = Rect::from(anchor);
    let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
    let padding = f32::from(placement.collision_padding);
    let gap = f32::from(placement.side_offset);
    let (w, h) = match side {
        Side::Top => (vw - 2. * padding, anchor.y - gap - padding),
        Side::Bottom => (
            vw - 2. * padding,
            vh - padding - (anchor.y + anchor.h + gap),
        ),
        Side::Left => (anchor.x - gap - padding, vh - 2. * padding),
        Side::Right => (
            vw - padding - (anchor.x + anchor.w + gap),
            vh - 2. * padding,
        ),
    };
    Size::new(px(w.max(0.)), px(h.max(0.)))
}

/// Where a surface of `size` goes on `side` with `align`, shifted inside the window but never
/// flipped: for a surface cut down to the room [`place`] chose.
pub fn origin_on(
    anchor: Bounds<Pixels>,
    size: Size<Pixels>,
    viewport: Size<Pixels>,
    side: Side,
    align: Align,
    placement: Placement,
) -> Point<Pixels> {
    let anchor = Rect::from(anchor);
    let (w, h) = (f32::from(size.width), f32::from(size.height));
    let (vw, vh) = (f32::from(viewport.width), f32::from(viewport.height));
    let rect = shift(
        raw(anchor, w, h, side, align, placement),
        anchor,
        side,
        vw,
        vh,
        placement,
    );
    point(px(rect.x), px(rect.y))
}

impl From<Bounds<Pixels>> for Rect {
    fn from(bounds: Bounds<Pixels>) -> Self {
        Self {
            x: bounds.origin.x.into(),
            y: bounds.origin.y.into(),
            w: bounds.size.width.into(),
            h: bounds.size.height.into(),
        }
    }
}

/// The order Floating UI's `flip` tries: the alignment flipped, the opposite side, then the
/// perpendicular sides.
fn candidates(side: Side, align: Align, fallback_axis: bool) -> Vec<(Side, Align)> {
    let [first, second] = side.perpendicular();
    let mut all = vec![
        (side, align),
        (side, align.flipped()),
        (side.opposite(), align),
        (side.opposite(), align.flipped()),
        (first, align),
        (second, align),
        (first, align.flipped()),
        (second, align.flipped()),
    ];
    if !fallback_axis {
        all.truncate(4);
    }
    let mut seen = Vec::with_capacity(all.len());
    all.retain(|candidate| {
        let new = !seen.contains(candidate);
        seen.push(*candidate);
        new
    });
    all
}

/// No candidate fits: the one whose side fits with the least overflow across, or else the one
/// that overflows least in all.
fn settle(tried: &[Tried], preferred: Side) -> Option<(Side, Align)> {
    let side_fits = tried
        .iter()
        .filter(|tried| tried.main <= 0.)
        .min_by(|a, b| a.cross[0].total_cmp(&b.cross[0]));
    if let Some(tried) = side_fits {
        return Some((tried.side, tried.align));
    }
    tried
        .iter()
        .filter(|tried| {
            tried.side.is_vertical() == preferred.is_vertical() || tried.side.is_vertical()
        })
        .map(|tried| {
            let total: f32 = std::iter::once(tried.main)
                .chain(tried.cross)
                .filter(|overflow| *overflow > 0.)
                .sum();
            (tried.side, tried.align, total)
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(side, align, _)| (side, align))
}

fn raw(anchor: Rect, w: f32, h: f32, side: Side, align: Align, placement: Placement) -> Rect {
    let side_offset = f32::from(placement.side_offset);
    let align_offset = match align {
        Align::End => -f32::from(placement.align_offset),
        _ => f32::from(placement.align_offset),
    };
    let aligned = |start: f32, anchor_length: f32, length: f32| {
        align_offset
            + match align {
                Align::Start => start,
                Align::Center => start + (anchor_length - length) / 2.,
                Align::End => start + anchor_length - length,
            }
    };
    match side {
        Side::Top => Rect {
            x: aligned(anchor.x, anchor.w, w),
            y: anchor.y - h - side_offset,
            w,
            h,
        },
        Side::Bottom => Rect {
            x: aligned(anchor.x, anchor.w, w),
            y: anchor.y + anchor.h + side_offset,
            w,
            h,
        },
        Side::Left => Rect {
            x: anchor.x - w - side_offset,
            y: aligned(anchor.y, anchor.h, h),
            w,
            h,
        },
        Side::Right => Rect {
            x: anchor.x + anchor.w + side_offset,
            y: aligned(anchor.y, anchor.h, h),
            w,
            h,
        },
    }
}

/// Moves the surface along its anchor's edge to keep it inside the window, but never so far
/// that the arrow could no longer point at the anchor.
fn shift(rect: Rect, anchor: Rect, side: Side, vw: f32, vh: f32, placement: Placement) -> Rect {
    let padding = f32::from(placement.collision_padding);
    let limit = if placement.arrow > px(0.) {
        f32::from(placement.arrow) / 2. + padding
    } else {
        0.
    };
    let settle = |start: f32, length: f32, viewport: f32, anchor_start: f32, anchor_length: f32| {
        let inside = start.min(viewport - padding - length).max(padding);
        inside
            .min(anchor_start + anchor_length - limit)
            .max(anchor_start - length + limit)
    };
    if side.is_vertical() {
        Rect {
            x: settle(rect.x, rect.w, vw, anchor.x, anchor.w),
            ..rect
        }
    } else {
        Rect {
            y: settle(rect.y, rect.h, vh, anchor.y, anchor.h),
            ..rect
        }
    }
}

fn overflow(rect: Rect, vw: f32, vh: f32, padding: Edges) -> Edges {
    Edges {
        top: padding.top - rect.y,
        right: rect.x + rect.w - (vw - padding.right),
        bottom: rect.y + rect.h - (vh - padding.bottom),
        left: padding.left - rect.x,
    }
}

#[cfg(test)]
mod tests {
    use super::{Align, Placed, Placement, Side, place};
    use gpui_kit::{Bounds, point, px, size};

    const VIEWPORT: (f32, f32) = (800., 600.);

    fn at(anchor: (f32, f32, f32, f32), popup: (f32, f32), placement: Placement) -> Placed {
        place(
            Bounds::new(
                point(px(anchor.0), px(anchor.1)),
                size(px(anchor.2), px(anchor.3)),
            ),
            size(px(popup.0), px(popup.1)),
            size(px(VIEWPORT.0), px(VIEWPORT.1)),
            placement,
        )
    }

    fn asked(side: Side, align: Align) -> Placement {
        Placement {
            side,
            align,
            side_offset: px(4.),
            arrow: px(10.),
            ..Placement::default()
        }
    }

    #[test]
    fn a_surface_that_fits_goes_where_asked() {
        let placed = at(
            (400., 300., 40., 20.),
            (100., 28.),
            asked(Side::Top, Align::Center),
        );
        assert_eq!(placed.side, Side::Top);
        assert_eq!(placed.origin, point(px(370.), px(268.)));
        assert_eq!(placed.arrow_center, px(50.));

        let placed = at(
            (400., 300., 40., 20.),
            (100., 28.),
            asked(Side::Right, Align::Start),
        );
        assert_eq!(placed.origin, point(px(444.), px(300.)));
        let placed = at(
            (400., 300., 40., 20.),
            (100., 28.),
            asked(Side::Bottom, Align::End),
        );
        assert_eq!(placed.origin, point(px(340.), px(324.)));
        let placed = at(
            (400., 300., 40., 20.),
            (100., 28.),
            asked(Side::Left, Align::Center),
        );
        assert_eq!(placed.origin, point(px(296.), px(296.)));
    }

    #[test]
    fn the_align_offset_moves_away_from_the_aligned_edge() {
        let start = Placement {
            align_offset: px(6.),
            ..asked(Side::Bottom, Align::Start)
        };
        let end = Placement {
            align_offset: px(6.),
            ..asked(Side::Bottom, Align::End)
        };
        let anchor = (400., 300., 40., 20.);
        assert_eq!(at(anchor, (100., 28.), start).origin.x, px(406.));
        assert_eq!(at(anchor, (100., 28.), end).origin.x, px(334.));
    }

    #[test]
    fn a_surface_with_no_room_flips_to_the_opposite_side() {
        let placed = at(
            (400., 10., 40., 20.),
            (100., 28.),
            asked(Side::Top, Align::Center),
        );
        assert_eq!(placed.side, Side::Bottom);
        assert_eq!(placed.origin.y, px(34.));

        let placed = at(
            (760., 300., 30., 20.),
            (100., 28.),
            asked(Side::Right, Align::Center),
        );
        assert_eq!(placed.side, Side::Left);
        assert_eq!(placed.origin.x, px(656.));
    }

    #[test]
    fn an_aligned_surface_flips_its_alignment_before_its_side() {
        let placed = at(
            (740., 300., 40., 20.),
            (120., 28.),
            asked(Side::Top, Align::Start),
        );
        assert_eq!((placed.side, placed.align), (Side::Top, Align::End));
        assert_eq!(placed.origin.x, px(660.));
    }

    #[test]
    fn a_centered_surface_shifts_inside_the_window_and_its_arrow_still_points_at_the_anchor() {
        let placed = at(
            (4., 300., 24., 24.),
            (160., 28.),
            asked(Side::Top, Align::Center),
        );
        assert_eq!(placed.side, Side::Top);
        assert_eq!(placed.origin.x, px(5.));
        assert_eq!(placed.arrow_center, px(11.));
    }

    #[test]
    fn the_shift_stops_where_the_arrow_would_leave_the_anchor() {
        let placed = at(
            (-30., 300., 20., 20.),
            (100., 28.),
            asked(Side::Bottom, Align::Center),
        );
        assert_eq!(placed.origin.x, px(-20.));
        assert_eq!(placed.arrow_center, px(10.));
    }

    #[test]
    fn a_surface_with_no_room_above_or_below_moves_to_a_side() {
        let placement = asked(Side::Top, Align::Center);
        let tall = place(
            Bounds::new(point(px(100.), px(40.)), size(px(40.), px(40.))),
            size(px(100.), px(60.)),
            size(px(800.), px(120.)),
            placement,
        );
        assert_eq!(tall.side, Side::Right);
    }

    #[test]
    fn with_no_room_anywhere_the_side_that_fits_wins() {
        let placed = place(
            Bounds::new(point(px(10.), px(10.)), size(px(20.), px(20.))),
            size(px(100.), px(28.)),
            size(px(110.), px(80.)),
            asked(Side::Top, Align::Center),
        );
        assert_eq!(placed.side, Side::Bottom);
    }
}
