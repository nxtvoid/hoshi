export type Point = { x: number; y: number }
export type Size = { width: number; height: number }
export type Frame = Point & Size

export type ResizeDirection =
  | 'north'
  | 'south'
  | 'east'
  | 'west'
  | 'northeast'
  | 'northwest'
  | 'southeast'
  | 'southwest'

type Span = { left: number; right: number }

/** What the floating browser keeps clear of, in the chat's own coordinates. */
export type Obstacles = {
  composer: (Span & { height: number }) | null
  detailsCard: (Span & { bottom: number }) | null
}

export const EDGE_GAP = 12
const DEFAULT_BOX: Size = { width: 320, height: 320 }
const MIN_SIZE: Size = { width: 240, height: 150 }
const NO_OBSTACLES: Obstacles = { composer: null, detailsCard: null }

const spanOf = (x: number, width: number): Span => ({
  left: x,
  right: x + width
})

const overlaps = (a: Span, b: Span) => a.left < b.right && a.right > b.left

function floorFor(span: Span, container: Size, obstacles: Obstacles) {
  const { composer } = obstacles
  return composer && overlaps(span, composer)
    ? container.height - Math.max(0, composer.height)
    : container.height
}

function ceilingFor(span: Span, obstacles: Obstacles) {
  const { detailsCard } = obstacles
  return detailsCard && overlaps(span, detailsCard)
    ? Math.max(0, detailsCard.bottom)
    : 0
}

function availableArea(
  container: Size,
  obstacles: Obstacles,
  span: Span | null
): Size {
  const height = span
    ? floorFor(span, container, obstacles) - ceilingFor(span, obstacles)
    : container.height - Math.max(0, obstacles.composer?.height ?? 0)
  return {
    width: container.width - EDGE_GAP * 2,
    height: height - EDGE_GAP * 2
  }
}

const minimumWidth = (source: Size) =>
  Math.max(MIN_SIZE.width, (MIN_SIZE.height * source.width) / source.height)

function fitWidth(
  desired: number,
  source: Size,
  max: Size,
  least = minimumWidth(source)
): Size {
  const ratio = source.width / source.height
  const width = Math.min(
    Math.max(desired, least),
    Math.max(1, max.width),
    Math.max(1, max.height * ratio)
  )
  return { width: Math.round(width), height: Math.round(width / ratio) }
}

const defaultWidth = (source: Size) =>
  Math.min(
    DEFAULT_BOX.width,
    (DEFAULT_BOX.height * source.width) / source.height
  )

function clampToContainer(
  position: Point,
  container: Size,
  player: Size,
  minimumX = EDGE_GAP
): Point {
  return {
    x: Math.min(
      Math.max(position.x, minimumX, EDGE_GAP),
      Math.max(EDGE_GAP, container.width - player.width - EDGE_GAP)
    ),
    y: Math.min(
      Math.max(position.y, EDGE_GAP),
      Math.max(EDGE_GAP, container.height - player.height - EDGE_GAP)
    )
  }
}

function hitsObstacle(
  position: Point,
  player: Size,
  container: Size,
  obstacles: Obstacles
) {
  const span = spanOf(position.x, player.width)
  return (
    position.y + player.height > floorFor(span, container, obstacles) ||
    position.y < ceilingFor(span, obstacles)
  )
}

/**
 * Keeps the player inside the chat and off the composer and the details card: above or
 * below them in the same column, else beside them, whichever is nearest.
 */
export function clampPosition(
  position: Point,
  container: Size,
  player: Size,
  obstacles: Obstacles = NO_OBSTACLES,
  minimumX = EDGE_GAP
): Point {
  const inside = clampToContainer(position, container, player, minimumX)
  if (!hitsObstacle(inside, player, container, obstacles)) return inside
  const fitRows = (x: number): Point => {
    const span = spanOf(x, player.width)
    return {
      x,
      y: Math.min(
        container.height - EDGE_GAP - player.height,
        Math.max(
          Math.min(
            inside.y,
            floorFor(span, container, obstacles) - EDGE_GAP - player.height
          ),
          ceilingFor(span, obstacles) + EDGE_GAP,
          EDGE_GAP
        )
      )
    }
  }
  const fallback = fitRows(inside.x)
  const span = spanOf(inside.x, player.width)
  const beside = [obstacles.composer, obstacles.detailsCard]
    .filter(
      (obstacle): obstacle is NonNullable<typeof obstacle> =>
        obstacle !== null && overlaps(span, obstacle)
    )
    .flatMap((obstacle) => [
      obstacle.left - EDGE_GAP - player.width,
      obstacle.right + EDGE_GAP
    ])
  let best = fallback
  let bestDistance = hitsObstacle(fallback, player, container, obstacles)
    ? Number.POSITIVE_INFINITY
    : Math.abs(fallback.y - inside.y)
  for (const x of beside) {
    const candidate = fitRows(x)
    if (
      clampToContainer(candidate, container, player, minimumX).x !== x ||
      hitsObstacle(candidate, player, container, obstacles)
    ) {
      continue
    }
    const distance = Math.abs(x - inside.x) + Math.abs(candidate.y - inside.y)
    if (distance < bestDistance) {
      best = candidate
      bestDistance = distance
    }
  }
  return best
}

/**
 * Where the player shows: where the user left it, else under the details card or in the
 * chat's top right corner. Clamped for this layout only, so a narrow chat doesn't shrink
 * the width the user chose.
 */
export function resolveFrame(input: {
  width: number | null
  position: Point | null
  source: Size
  container: Size
  obstacles?: Obstacles
}): Frame {
  const { width, position, source, container } = input
  const obstacles = input.obstacles ?? NO_OBSTACLES
  const placed = position !== null && width !== null
  const size = fitWidth(
    width ?? defaultWidth(source),
    source,
    availableArea(
      container,
      placed ? { composer: obstacles.composer, detailsCard: null } : obstacles,
      placed ? spanOf(position.x, width) : null
    ),
    width ?? undefined
  )
  const { detailsCard } = obstacles
  const anchor =
    position ??
    (detailsCard
      ? { x: detailsCard.right - size.width, y: detailsCard.bottom + EDGE_GAP }
      : { x: container.width - EDGE_GAP - size.width, y: EDGE_GAP })
  return { ...clampPosition(anchor, container, size, obstacles), ...size }
}

/** The frame after dragging one of its edges or corners by `delta`, keeping the page's shape. */
export function resizeFrame(input: {
  start: Frame
  direction: ResizeDirection
  delta: Point
  source: Size
  container: Size
  obstacles?: Obstacles
}): Frame {
  const { start, direction, delta, source, container } = input
  const obstacles = input.obstacles ?? NO_OBSTACLES
  const east = direction.includes('east')
  const west = direction.includes('west')
  const north = direction.includes('north')
  const south = direction.includes('south')
  const right = start.x + start.width
  const bottom = start.y + start.height
  const span = spanOf(start.x, start.width)
  const floor = floorFor(span, container, obstacles)
  const ceiling = ceilingFor(span, obstacles)
  const max = {
    width: west
      ? right - EDGE_GAP
      : east
        ? container.width - EDGE_GAP - start.x
        : container.width - EDGE_GAP * 2,
    height: north
      ? bottom - ceiling - EDGE_GAP
      : south
        ? floor - EDGE_GAP - start.y
        : floor - ceiling - EDGE_GAP * 2
  }
  const desiredWidth = start.width + (east ? delta.x : west ? -delta.x : 0)
  const desiredHeight = start.height + (south ? delta.y : north ? -delta.y : 0)
  const horizontal = east || west
  const vertical = north || south
  const ratio = source.width / source.height
  // A corner projects onto the shape's diagonal, so opposite moves on each axis don't jump.
  const desired =
    horizontal && vertical
      ? (desiredWidth + desiredHeight / ratio) / (1 + 1 / ratio ** 2)
      : horizontal
        ? desiredWidth
        : desiredHeight * ratio
  const size = fitWidth(
    desired,
    source,
    max,
    Math.min(start.width, minimumWidth(source))
  )
  const position = clampPosition(
    {
      x: west ? right - size.width : start.x,
      y: north ? bottom - size.height : start.y
    },
    container,
    size,
    obstacles
  )
  return { ...position, ...size }
}

/** What the user did to the player last: a resize lets it crowd the chat, a drag doesn't. */
export type Interaction = 'drag' | 'resize'

/** The chat column, in the chat's own coordinates. */
export type ChatLane = { left: number; width: number }

const DETAILS_CLEARANCE = 32

/**
 * Where the player and the chat column go together. The player opens in the bottom right
 * corner, and the chat moves left to make room while it stays readable; when it can't, the
 * player floats over the chat, above the composer.
 */
export function resolveCanvas({
  container,
  player,
  padding = 48,
  maxChatWidth = 768,
  minChatWidth = 640,
  composerHeight = 0,
  detailsCard = null
}: {
  container: Size
  player: {
    width: number | null
    position: Point | null
    source: Size
    interaction: Interaction
  }
  padding?: number
  maxChatWidth?: number
  minChatWidth?: number
  composerHeight?: number
  detailsCard?: Obstacles['detailsCard']
}): { chat: ChatLane; frame: Frame; overlapsChat: boolean; moved: boolean } {
  const centeredWidth = Math.max(
    0,
    Math.min(maxChatWidth, container.width - padding * 2)
  )
  const laneRight = detailsCard
    ? detailsCard.left - DETAILS_CLEARANCE
    : container.width - padding
  const normalWidth = Math.max(0, Math.min(centeredWidth, laneRight - padding))
  const normalLeft = Math.max(
    padding,
    Math.min((container.width - normalWidth) / 2, laneRight - normalWidth)
  )
  const normal = { left: normalLeft, width: normalWidth }
  const resized = player.interaction === 'resize'
  let frame = resolveFrame({ ...player, container })
  // The chat keeps a readable lane while the player still has room to move beside it.
  const minimumX =
    resized ||
    container.width <= padding + minChatWidth + frame.width + EDGE_GAP * 2
      ? EDGE_GAP
      : padding + minChatWidth + EDGE_GAP
  if (player.position === null) {
    frame = { ...frame, y: container.height - frame.height - EDGE_GAP }
  }
  if (resized && composerHeight > 0) {
    const laneWidth = Math.min(normalWidth, minChatWidth)
    const transition = Math.max(EDGE_GAP, normalWidth - laneWidth)
    const lift = Math.min(
      1,
      Math.max(0, (padding + normalWidth + EDGE_GAP - frame.x) / transition)
    )
    if (lift > 0) {
      frame = resolveFrame({
        width: frame.width,
        position: frame,
        source: player.source,
        container: {
          ...container,
          height: Math.max(
            EDGE_GAP * 2 + 1,
            container.height - composerHeight * lift
          )
        }
      })
    }
  }
  const preferred = {
    ...frame,
    ...clampPosition(frame, container, frame, NO_OBSTACLES, minimumX)
  }
  const card = resized ? null : detailsCard
  frame = {
    ...frame,
    ...clampPosition(
      frame,
      container,
      frame,
      { detailsCard: card, composer: null },
      minimumX
    )
  }
  const chatBeside = (placed: Frame): ChatLane | null => {
    if (placed.x >= normalLeft + normalWidth + EDGE_GAP) return normal
    const width = Math.min(maxChatWidth, placed.x - EDGE_GAP - padding)
    if (width < minChatWidth) return null
    return { left: Math.min(normalLeft, placed.x - EDGE_GAP - width), width }
  }
  let chat = chatBeside(frame)
  if (!chat && detailsCard) {
    chat = chatBeside(preferred)
    if (chat) frame = preferred
  }
  if (chat) return { chat, frame, overlapsChat: false, moved: chat !== normal }
  if (resized) {
    return {
      chat: { left: padding, width: Math.min(normalWidth, minChatWidth) },
      frame,
      overlapsChat: true,
      moved: true
    }
  }
  const obstacles: Obstacles = {
    detailsCard: card,
    composer: {
      left: normalLeft,
      right: normalLeft + normalWidth,
      height: composerHeight
    }
  }
  frame = resolveFrame({ ...player, container, obstacles })
  frame = {
    ...frame,
    ...clampPosition(
      player.position ?? {
        x: frame.x,
        y: container.height - frame.height - EDGE_GAP
      },
      container,
      frame,
      obstacles,
      minimumX
    )
  }
  return { chat: normal, frame, overlapsChat: true, moved: false }
}
