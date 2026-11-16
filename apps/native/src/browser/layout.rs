//! Where the floating browser sits over the chat and what room the chat column leaves it, in
//! the chat's own coordinates.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Frame {
    fn at(position: Point, size: Size) -> Self {
        Self {
            x: position.x,
            y: position.y,
            width: size.width,
            height: size.height,
        }
    }

    pub fn position(&self) -> Point {
        Point {
            x: self.x,
            y: self.y,
        }
    }

    pub fn size(&self) -> Size {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn moved_to(self, position: Point) -> Self {
        Self::at(position, self.size())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeDirection {
    North,
    South,
    East,
    West,
    Northeast,
    Northwest,
    Southeast,
    Southwest,
}

impl ResizeDirection {
    pub const ALL: [Self; 8] = [
        Self::North,
        Self::South,
        Self::West,
        Self::East,
        Self::Northwest,
        Self::Northeast,
        Self::Southwest,
        Self::Southeast,
    ];

    fn north(self) -> bool {
        matches!(self, Self::North | Self::Northeast | Self::Northwest)
    }

    fn south(self) -> bool {
        matches!(self, Self::South | Self::Southeast | Self::Southwest)
    }

    fn east(self) -> bool {
        matches!(self, Self::East | Self::Northeast | Self::Southeast)
    }

    fn west(self) -> bool {
        matches!(self, Self::West | Self::Northwest | Self::Southwest)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Span {
    left: f64,
    right: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Composer {
    pub left: f64,
    pub right: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetailsCard {
    pub left: f64,
    pub right: f64,
    pub bottom: f64,
}

/// What the floating browser keeps clear of.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Obstacles {
    pub composer: Option<Composer>,
    pub details_card: Option<DetailsCard>,
}

pub const EDGE_GAP: f64 = 12.;
const DEFAULT_BOX: Size = Size {
    width: 320.,
    height: 320.,
};
const MIN_SIZE: Size = Size {
    width: 240.,
    height: 150.,
};
const NO_OBSTACLES: Obstacles = Obstacles {
    composer: None,
    details_card: None,
};
const PADDING: f64 = 48.;
const MAX_CHAT_WIDTH: f64 = 768.;
const MIN_CHAT_WIDTH: f64 = 640.;
const DETAILS_CLEARANCE: f64 = 32.;

/// `Math.round`: halves go up.
fn round(value: f64) -> f64 {
    (value + 0.5).floor()
}

fn span_of(x: f64, width: f64) -> Span {
    Span {
        left: x,
        right: x + width,
    }
}

fn overlaps(a: Span, b: Span) -> bool {
    a.left < b.right && a.right > b.left
}

impl Composer {
    fn span(&self) -> Span {
        Span {
            left: self.left,
            right: self.right,
        }
    }
}

impl DetailsCard {
    fn span(&self) -> Span {
        Span {
            left: self.left,
            right: self.right,
        }
    }
}

fn floor_for(span: Span, container: Size, obstacles: Obstacles) -> f64 {
    match obstacles.composer {
        Some(composer) if overlaps(span, composer.span()) => {
            container.height - composer.height.max(0.)
        }
        _ => container.height,
    }
}

fn ceiling_for(span: Span, obstacles: Obstacles) -> f64 {
    match obstacles.details_card {
        Some(card) if overlaps(span, card.span()) => card.bottom.max(0.),
        _ => 0.,
    }
}

fn available_area(container: Size, obstacles: Obstacles, span: Option<Span>) -> Size {
    let height = match span {
        Some(span) => floor_for(span, container, obstacles) - ceiling_for(span, obstacles),
        None => {
            container.height
                - obstacles
                    .composer
                    .map_or(0., |composer| composer.height)
                    .max(0.)
        }
    };
    Size {
        width: container.width - EDGE_GAP * 2.,
        height: height - EDGE_GAP * 2.,
    }
}

fn minimum_width(source: Size) -> f64 {
    MIN_SIZE
        .width
        .max(MIN_SIZE.height * source.width / source.height)
}

fn fit_width(desired: f64, source: Size, max: Size, least: f64) -> Size {
    let ratio = source.width / source.height;
    let width = desired
        .max(least)
        .min(max.width.max(1.))
        .min((max.height * ratio).max(1.));
    Size {
        width: round(width),
        height: round(width / ratio),
    }
}

fn default_width(source: Size) -> f64 {
    DEFAULT_BOX
        .width
        .min(DEFAULT_BOX.height * source.width / source.height)
}

fn clamp_to_container(position: Point, container: Size, player: Size, minimum_x: f64) -> Point {
    Point {
        x: position
            .x
            .max(minimum_x)
            .max(EDGE_GAP)
            .min(EDGE_GAP.max(container.width - player.width - EDGE_GAP)),
        y: position
            .y
            .max(EDGE_GAP)
            .min(EDGE_GAP.max(container.height - player.height - EDGE_GAP)),
    }
}

fn hits_obstacle(position: Point, player: Size, container: Size, obstacles: Obstacles) -> bool {
    let span = span_of(position.x, player.width);
    position.y + player.height > floor_for(span, container, obstacles)
        || position.y < ceiling_for(span, obstacles)
}

/// Keeps the player inside the chat and off the composer and the details card: above or
/// below them in the same column, else beside them, whichever is nearest.
pub fn clamp_position(
    position: Point,
    container: Size,
    player: Size,
    obstacles: Obstacles,
    minimum_x: f64,
) -> Point {
    let inside = clamp_to_container(position, container, player, minimum_x);
    if !hits_obstacle(inside, player, container, obstacles) {
        return inside;
    }
    let fit_rows = |x: f64| {
        let span = span_of(x, player.width);
        Point {
            x,
            y: (container.height - EDGE_GAP - player.height).min(
                inside
                    .y
                    .min(floor_for(span, container, obstacles) - EDGE_GAP - player.height)
                    .max(ceiling_for(span, obstacles) + EDGE_GAP)
                    .max(EDGE_GAP),
            ),
        }
    };
    let fallback = fit_rows(inside.x);
    let span = span_of(inside.x, player.width);
    let beside = [
        obstacles.composer.map(|composer| composer.span()),
        obstacles.details_card.map(|card| card.span()),
    ]
    .into_iter()
    .flatten()
    .filter(|obstacle| overlaps(span, *obstacle))
    .flat_map(|obstacle| {
        [
            obstacle.left - EDGE_GAP - player.width,
            obstacle.right + EDGE_GAP,
        ]
    });
    let mut best = fallback;
    let mut best_distance = if hits_obstacle(fallback, player, container, obstacles) {
        f64::INFINITY
    } else {
        (fallback.y - inside.y).abs()
    };
    for x in beside {
        let candidate = fit_rows(x);
        if clamp_to_container(candidate, container, player, minimum_x).x != x
            || hits_obstacle(candidate, player, container, obstacles)
        {
            continue;
        }
        let distance = (x - inside.x).abs() + (candidate.y - inside.y).abs();
        if distance < best_distance {
            best = candidate;
            best_distance = distance;
        }
    }
    best
}

/// Where the player shows: where the user left it, else under the details card or in the
/// chat's top right corner. Clamped for this layout only, so a narrow chat doesn't shrink
/// the width the user chose.
pub fn resolve_frame(
    width: Option<f64>,
    position: Option<Point>,
    source: Size,
    container: Size,
    obstacles: Obstacles,
) -> Frame {
    let placed = match (position, width) {
        (Some(position), Some(width)) => Some(span_of(position.x, width)),
        _ => None,
    };
    let area_obstacles = if placed.is_some() {
        Obstacles {
            composer: obstacles.composer,
            details_card: None,
        }
    } else {
        obstacles
    };
    let size = fit_width(
        width.unwrap_or_else(|| default_width(source)),
        source,
        available_area(container, area_obstacles, placed),
        width.unwrap_or_else(|| minimum_width(source)),
    );
    let anchor = position.unwrap_or(match obstacles.details_card {
        Some(card) => Point {
            x: card.right - size.width,
            y: card.bottom + EDGE_GAP,
        },
        None => Point {
            x: container.width - EDGE_GAP - size.width,
            y: EDGE_GAP,
        },
    });
    Frame::at(
        clamp_position(anchor, container, size, obstacles, EDGE_GAP),
        size,
    )
}

/// The frame after dragging one of its edges or corners by `delta`, keeping the page's shape.
pub fn resize_frame(
    start: Frame,
    direction: ResizeDirection,
    delta: Point,
    source: Size,
    container: Size,
    obstacles: Obstacles,
) -> Frame {
    let (east, west) = (direction.east(), direction.west());
    let (north, south) = (direction.north(), direction.south());
    let right = start.x + start.width;
    let bottom = start.y + start.height;
    let span = span_of(start.x, start.width);
    let floor = floor_for(span, container, obstacles);
    let ceiling = ceiling_for(span, obstacles);
    let max = Size {
        width: if west {
            right - EDGE_GAP
        } else if east {
            container.width - EDGE_GAP - start.x
        } else {
            container.width - EDGE_GAP * 2.
        },
        height: if north {
            bottom - ceiling - EDGE_GAP
        } else if south {
            floor - EDGE_GAP - start.y
        } else {
            floor - ceiling - EDGE_GAP * 2.
        },
    };
    let desired_width = start.width
        + if east {
            delta.x
        } else if west {
            -delta.x
        } else {
            0.
        };
    let desired_height = start.height
        + if south {
            delta.y
        } else if north {
            -delta.y
        } else {
            0.
        };
    let horizontal = east || west;
    let vertical = north || south;
    let ratio = source.width / source.height;
    // A corner projects onto the shape's diagonal, so opposite moves on each axis don't jump.
    let desired = if horizontal && vertical {
        (desired_width + desired_height / ratio) / (1. + 1. / ratio.powi(2))
    } else if horizontal {
        desired_width
    } else {
        desired_height * ratio
    };
    let size = fit_width(desired, source, max, start.width.min(minimum_width(source)));
    let position = clamp_position(
        Point {
            x: if west { right - size.width } else { start.x },
            y: if north { bottom - size.height } else { start.y },
        },
        container,
        size,
        obstacles,
        EDGE_GAP,
    );
    Frame::at(position, size)
}

/// What the user did to the player last: a resize lets it crowd the chat, a drag doesn't.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interaction {
    #[default]
    Drag,
    Resize,
}

/// Where the user left a thread's floating browser; `None` until they move or resize it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Placement {
    pub position: Option<Point>,
    pub width: Option<f64>,
    pub interaction: Interaction,
}

/// The chat column.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChatLane {
    pub left: f64,
    pub width: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Canvas {
    pub chat: ChatLane,
    pub frame: Frame,
    pub overlaps_chat: bool,
    /// The chat column left its centered place to make room.
    pub moved: bool,
}

/// Where the player and the chat column go together. The player opens in the bottom right
/// corner, and the chat moves left to make room while it stays readable; when it can't, the
/// player floats over the chat, above the composer.
pub fn resolve_canvas(
    container: Size,
    player: Placement,
    source: Size,
    composer_height: f64,
    details_card: Option<DetailsCard>,
) -> Canvas {
    let centered_width = MAX_CHAT_WIDTH.min(container.width - PADDING * 2.).max(0.);
    let lane_right = details_card.map_or(container.width - PADDING, |card| {
        card.left - DETAILS_CLEARANCE
    });
    let normal_width = centered_width.min(lane_right - PADDING).max(0.);
    let normal_left =
        PADDING.max(((container.width - normal_width) / 2.).min(lane_right - normal_width));
    let normal = ChatLane {
        left: normal_left,
        width: normal_width,
    };
    let resized = player.interaction == Interaction::Resize;
    let mut frame = resolve_frame(
        player.width,
        player.position,
        source,
        container,
        NO_OBSTACLES,
    );
    // The chat keeps a readable lane while the player still has room to move beside it.
    let minimum_x =
        if resized || container.width <= PADDING + MIN_CHAT_WIDTH + frame.width + EDGE_GAP * 2. {
            EDGE_GAP
        } else {
            PADDING + MIN_CHAT_WIDTH + EDGE_GAP
        };
    if player.position.is_none() {
        frame.y = container.height - frame.height - EDGE_GAP;
    }
    if resized && composer_height > 0. {
        let lane_width = normal_width.min(MIN_CHAT_WIDTH);
        let transition = EDGE_GAP.max(normal_width - lane_width);
        let lift = ((PADDING + normal_width + EDGE_GAP - frame.x) / transition).clamp(0., 1.);
        if lift > 0. {
            frame = resolve_frame(
                Some(frame.width),
                Some(frame.position()),
                source,
                Size {
                    width: container.width,
                    height: (EDGE_GAP * 2. + 1.).max(container.height - composer_height * lift),
                },
                NO_OBSTACLES,
            );
        }
    }
    let preferred = frame.moved_to(clamp_position(
        frame.position(),
        container,
        frame.size(),
        NO_OBSTACLES,
        minimum_x,
    ));
    let card = if resized { None } else { details_card };
    frame = frame.moved_to(clamp_position(
        frame.position(),
        container,
        frame.size(),
        Obstacles {
            composer: None,
            details_card: card,
        },
        minimum_x,
    ));
    let chat_beside = |placed: Frame| -> Option<(ChatLane, bool)> {
        if placed.x >= normal_left + normal_width + EDGE_GAP {
            return Some((normal, true));
        }
        let width = MAX_CHAT_WIDTH.min(placed.x - EDGE_GAP - PADDING);
        if width < MIN_CHAT_WIDTH {
            return None;
        }
        Some((
            ChatLane {
                left: normal_left.min(placed.x - EDGE_GAP - width),
                width,
            },
            false,
        ))
    };
    let mut chat = chat_beside(frame);
    if chat.is_none() && details_card.is_some() {
        chat = chat_beside(preferred);
        if chat.is_some() {
            frame = preferred;
        }
    }
    if let Some((chat, normal)) = chat {
        return Canvas {
            chat,
            frame,
            overlaps_chat: false,
            moved: !normal,
        };
    }
    if resized {
        return Canvas {
            chat: ChatLane {
                left: PADDING,
                width: normal_width.min(MIN_CHAT_WIDTH),
            },
            frame,
            overlaps_chat: true,
            moved: true,
        };
    }
    let obstacles = Obstacles {
        details_card: card,
        composer: Some(Composer {
            left: normal_left,
            right: normal_left + normal_width,
            height: composer_height,
        }),
    };
    frame = resolve_frame(player.width, player.position, source, container, obstacles);
    let anchor = player.position.unwrap_or(Point {
        x: frame.x,
        y: container.height - frame.height - EDGE_GAP,
    });
    frame = frame.moved_to(clamp_position(
        anchor,
        container,
        frame.size(),
        obstacles,
        minimum_x,
    ));
    Canvas {
        chat: normal,
        frame,
        overlaps_chat: true,
        moved: false,
    }
}

/// What the chat column leaves at its left and right while the player moves it aside.
pub fn lane_room(canvas: &Canvas, container: Size) -> Option<(f64, f64)> {
    canvas.moved.then(|| {
        (
            round(canvas.chat.left),
            round(container.width - canvas.chat.left - canvas.chat.width),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{
        Composer, DetailsCard, EDGE_GAP, Frame, Interaction, Obstacles, Placement, Point,
        ResizeDirection, Size, clamp_position, resize_frame, resolve_canvas, resolve_frame,
    };

    const CONTAINER: Size = Size {
        width: 1000.,
        height: 700.,
    };
    const DESKTOP: Size = Size {
        width: 1280.,
        height: 800.,
    };
    const COMPOSER: Composer = Composer {
        left: 200.,
        right: 800.,
        height: 120.,
    };
    const DETAILS_CARD: DetailsCard = DetailsCard {
        left: 700.,
        right: 988.,
        bottom: 200.,
    };
    const NONE: Obstacles = Obstacles {
        composer: None,
        details_card: None,
    };

    fn size(width: f64, height: f64) -> Size {
        Size { width, height }
    }

    fn point(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    #[test]
    fn opens_in_the_top_right_corner_at_the_page_shape() {
        let frame = resolve_frame(None, None, DESKTOP, CONTAINER, NONE);
        assert_eq!(
            frame,
            Frame {
                x: CONTAINER.width - EDGE_GAP - 320.,
                y: EDGE_GAP,
                width: 320.,
                height: 200.,
            }
        );
    }

    #[test]
    fn a_phone_page_opens_at_the_minimum_width_at_its_shape() {
        let frame = resolve_frame(None, None, size(390., 844.), CONTAINER, NONE);
        assert_eq!(frame.width, 240.);
        assert_eq!(frame.height, (240. * 844. / 390_f64).round());
    }

    #[test]
    fn opens_under_the_details_card_flush_with_its_right_edge() {
        let obstacles = Obstacles {
            composer: None,
            details_card: Some(DETAILS_CARD),
        };
        let frame = resolve_frame(None, None, DESKTOP, CONTAINER, obstacles);
        assert_eq!(frame.x + frame.width, DETAILS_CARD.right);
        assert_eq!(frame.y, DETAILS_CARD.bottom + EDGE_GAP);
    }

    #[test]
    fn keeps_the_width_the_user_chose_and_stays_inside_a_narrow_chat() {
        let frame = resolve_frame(
            Some(600.),
            Some(point(380., 40.)),
            DESKTOP,
            size(500., 700.),
            NONE,
        );
        assert!(frame.width <= 500. - EDGE_GAP * 2.);
        assert!(frame.x >= EDGE_GAP);
    }

    const PLAYER: Size = Size {
        width: 320.,
        height: 200.,
    };
    const WITH_COMPOSER: Obstacles = Obstacles {
        composer: Some(COMPOSER),
        details_card: None,
    };

    #[test]
    fn lifts_the_player_off_the_composer() {
        let position = clamp_position(
            point(300., 600.),
            CONTAINER,
            PLAYER,
            WITH_COMPOSER,
            EDGE_GAP,
        );
        assert!(position.y + PLAYER.height <= CONTAINER.height - COMPOSER.height - EDGE_GAP);
    }

    #[test]
    fn a_column_beside_the_composer_keeps_its_place() {
        let position = clamp_position(
            point(820., 480.),
            size(1200., 700.),
            PLAYER,
            WITH_COMPOSER,
            EDGE_GAP,
        );
        assert_eq!(position, point(820., 480.));
    }

    #[test]
    fn stays_inside_the_chat() {
        assert_eq!(
            clamp_position(point(-50., -50.), CONTAINER, PLAYER, NONE, EDGE_GAP),
            point(EDGE_GAP, EDGE_GAP)
        );
    }

    const START: Frame = Frame {
        x: 400.,
        y: 100.,
        width: 320.,
        height: 200.,
    };

    #[test]
    fn dragging_the_east_edge_grows_the_frame_at_the_page_shape() {
        let frame = resize_frame(
            START,
            ResizeDirection::East,
            point(160., 0.),
            DESKTOP,
            CONTAINER,
            NONE,
        );
        assert_eq!(
            frame,
            Frame {
                x: 400.,
                y: 100.,
                width: 480.,
                height: 300.,
            }
        );
    }

    #[test]
    fn the_west_edge_keeps_the_right_side_in_place() {
        let frame = resize_frame(
            START,
            ResizeDirection::West,
            point(-80., 0.),
            DESKTOP,
            CONTAINER,
            NONE,
        );
        assert_eq!(frame.x + frame.width, START.x + START.width);
        assert_eq!(frame.width, 400.);
    }

    #[test]
    fn never_shrinks_below_the_minimum() {
        let frame = resize_frame(
            START,
            ResizeDirection::Southeast,
            point(-500., -500.),
            DESKTOP,
            CONTAINER,
            NONE,
        );
        assert_eq!(frame.width, 240.);
        assert_eq!(frame.height, 150.);
    }

    const OPENED: Placement = Placement {
        position: None,
        width: None,
        interaction: Interaction::Drag,
    };

    #[test]
    fn opens_in_the_bottom_right_corner_and_moves_the_chat_aside() {
        let wide = size(1400., 800.);
        let canvas = resolve_canvas(wide, OPENED, DESKTOP, 120., None);
        let frame = canvas.frame;
        assert_eq!(frame.x + frame.width, wide.width - EDGE_GAP);
        assert_eq!(frame.y + frame.height, wide.height - EDGE_GAP);
        assert!(!canvas.overlaps_chat);
        assert!(canvas.moved);
        assert!(canvas.chat.left + canvas.chat.width <= frame.x - EDGE_GAP);
        assert!(canvas.chat.width >= 640.);
    }

    #[test]
    fn leaves_the_chat_alone_when_the_player_already_clears_it() {
        let canvas = resolve_canvas(size(1900., 800.), OPENED, DESKTOP, 120., None);
        assert!(!canvas.moved);
    }

    #[test]
    fn in_a_narrow_chat_it_floats_over_the_chat_above_the_composer() {
        let canvas = resolve_canvas(CONTAINER, OPENED, DESKTOP, 120., None);
        assert!(canvas.overlaps_chat);
        assert!(!canvas.moved);
        assert!(canvas.frame.y + canvas.frame.height <= CONTAINER.height - 120. - EDGE_GAP);
    }

    #[test]
    fn a_drag_stops_where_the_chat_would_turn_unreadable() {
        let player = Placement {
            width: Some(320.),
            position: Some(point(100., 300.)),
            ..OPENED
        };
        let canvas = resolve_canvas(size(1400., 800.), player, DESKTOP, 120., None);
        assert_eq!(canvas.frame.x, 48. + 640. + EDGE_GAP);
    }

    #[test]
    fn the_room_is_what_the_moved_chat_leaves_on_each_side() {
        let wide = size(1400., 800.);
        let canvas = resolve_canvas(wide, OPENED, DESKTOP, 120., None);
        let (left, right) = super::lane_room(&canvas, wide).unwrap();
        assert_eq!(left, canvas.chat.left.round());
        assert_eq!(left + canvas.chat.width + right, wide.width);
        let still = resolve_canvas(size(1900., 800.), OPENED, DESKTOP, 120., None);
        assert_eq!(super::lane_room(&still, size(1900., 800.)), None);
    }
}
