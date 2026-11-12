import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { usePanelStore } from '@/state/panel-store'
import { useBrowserStore } from '@/state/browser-store'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { useMiniBrowserStore } from '@/state/mini-browser-store'
import { Cancel01Icon, SidebarRightIcon } from '@hugeicons/core-free-icons'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { BrowserPage, useAgentActive, useBrowserViewing } from './BrowserPage'
import {
  clampPosition,
  type Frame,
  type Obstacles,
  type ResizeDirection,
  resizeFrame,
  resolveCanvas,
  type Size
} from '@/lib/mini-browser-layout'

/** Marks what the floating browser keeps clear of inside the chat. */
export const OBSTACLE = 'data-mini-browser-obstacle'

const DEFAULT_PAGE: Size = { width: 1280, height: 800 }
/** How far a press on the handle moves before it counts as a drag. */
const TAP_SLOP_PX = 6
const PILL_INSET = 8

const RESIZE_HANDLES: { direction: ResizeDirection; className: string }[] = [
  { direction: 'north', className: 'inset-x-0 -top-1 h-2 cursor-ns-resize' },
  { direction: 'south', className: 'inset-x-0 -bottom-1 h-2 cursor-ns-resize' },
  { direction: 'west', className: 'inset-y-0 -left-1 w-2 cursor-ew-resize' },
  { direction: 'east', className: 'inset-y-0 -right-1 w-2 cursor-ew-resize' },
  {
    direction: 'northwest',
    className: '-top-2 -left-2 size-4 cursor-nwse-resize'
  },
  {
    direction: 'northeast',
    className: '-top-2 -right-2 size-4 cursor-nesw-resize'
  },
  {
    direction: 'southwest',
    className: '-bottom-2 -left-2 size-4 cursor-nesw-resize'
  },
  {
    direction: 'southeast',
    className: '-right-2 -bottom-2 size-4 cursor-nwse-resize'
  }
]

type Gesture = {
  pointerId: number
  pointerType: string
  x: number
  y: number
  frame: Frame
  direction: ResizeDirection | null
  moved: boolean
}

type ChatLayout = {
  container: Size
  composerHeight: number
  detailsCard: Obstacles['detailsCard']
}

const NO_LAYOUT: ChatLayout = {
  container: { width: 0, height: 0 },
  composerHeight: 0,
  detailsCard: null
}

function measure(chat: HTMLElement): ChatLayout {
  const box = chat.getBoundingClientRect()
  const obstacle = (kind: string) =>
    chat.querySelector<HTMLElement>(`[${OBSTACLE}='${kind}']`)
  const composer = obstacle('composer')?.getBoundingClientRect()
  const details = obstacle('details')?.getBoundingClientRect()
  return {
    container: { width: box.width, height: box.height },
    composerHeight:
      composer && composer.width > 0 ? box.bottom - composer.top : 0,
    detailsCard:
      details && details.width > 0
        ? {
            left: details.left - box.left,
            right: details.right - box.left,
            bottom: details.bottom - box.top
          }
        : null
  }
}

const sameLayout = (a: ChatLayout, b: ChatLayout) =>
  JSON.stringify(a) === JSON.stringify(b)

/**
 * The chat's size and what lies over it, followed as they resize. `obstaclesKey` changes
 * when one comes or goes.
 */
function useChatLayout(
  chat: React.RefObject<HTMLElement | null>,
  obstaclesKey: string
) {
  const [layout, setLayout] = useState(NO_LAYOUT)
  useLayoutEffect(() => {
    const element = chat.current
    if (!element) return
    const update = () => {
      const next = measure(element)
      setLayout((current) => (sameLayout(current, next) ? current : next))
    }
    const resizes = new ResizeObserver(update)
    resizes.observe(element)
    for (const obstacle of element.querySelectorAll(`[${OBSTACLE}]`)) {
      resizes.observe(obstacle)
    }
    update()
    return () => resizes.disconnect()
  }, [chat, obstaclesKey])
  return layout
}

function usePanelShowsBrowser(threadId: string) {
  return usePanelStore((state) => {
    if (!state.openThreads[threadId]) return false
    const panel = state.threads[threadId]
    return (
      panel?.surfaces.find((surface) => surface.id === panel.activeId)?.kind ===
      'browser'
    )
  })
}

/**
 * The thread's browser page floating over the chat, where it opens when the agent opens
 * one: moved by its handle, resized by its edges, and sent to the side panel from there.
 */
export function MiniBrowser({
  threadId,
  chat,
  obstaclesKey,
  hidden
}: {
  threadId: string
  chat: React.RefObject<HTMLElement | null>
  /** Changes when the details card or the composer comes or goes. */
  obstaclesKey: string
  /** The chat it floats over is hidden, so its page needs no frames. */
  hidden: boolean
}) {
  const placed = useMiniBrowserStore((state) => state.threads[threadId])
  const pageOpen = useBrowserStore((state) =>
    Boolean(state.pages[threadId]?.open)
  )
  const inPanel = usePanelShowsBrowser(threadId)
  if (!placed || !pageOpen || inPanel || hidden) return null
  return <Player threadId={threadId} chat={chat} obstaclesKey={obstaclesKey} />
}

function Player({
  threadId,
  chat,
  obstaclesKey
}: {
  threadId: string
  chat: React.RefObject<HTMLElement | null>
  obstaclesKey: string
}) {
  const placed = useMiniBrowserStore((state) => state.threads[threadId])
  const pageWidth = useBrowserStore((state) => state.frames[threadId]?.width)
  const pageHeight = useBrowserStore((state) => state.frames[threadId]?.height)
  const agentActive = useAgentActive(threadId)
  const { container, composerHeight, detailsCard } = useChatLayout(
    chat,
    obstaclesKey
  )
  const gesture = useRef<Gesture | null>(null)
  const [pillOpen, setPillOpen] = useState(false)
  const handle = useRef<HTMLDivElement>(null)
  useBrowserViewing(threadId, true)

  useEffect(() => {
    if (!pillOpen) return
    const dismiss = (event: PointerEvent) => {
      if (
        event.target instanceof Node &&
        handle.current?.contains(event.target)
      )
        return
      setPillOpen(false)
    }
    document.addEventListener('pointerdown', dismiss, true)
    return () => document.removeEventListener('pointerdown', dismiss, true)
  }, [pillOpen])

  const source =
    pageWidth && pageHeight
      ? { width: pageWidth, height: pageHeight }
      : DEFAULT_PAGE
  const canvas =
    placed && container.width > 0
      ? resolveCanvas({
          container,
          player: {
            width: placed.width,
            position: placed.position,
            source,
            interaction: placed.interaction
          },
          composerHeight,
          detailsCard
        })
      : null
  const moved = canvas?.moved === true ? canvas.chat : null
  const roomLeft = moved ? Math.round(moved.left) : null
  const roomRight = moved
    ? Math.round(container.width - moved.left - moved.width)
    : null

  useLayoutEffect(() => {
    const { setRoom } = useMiniBrowserStore.getState()
    setRoom(
      roomLeft === null || roomRight === null
        ? null
        : { threadId, left: roomLeft, right: roomRight }
    )
  }, [threadId, roomLeft, roomRight])

  useEffect(
    () => () => {
      const { room, setRoom } = useMiniBrowserStore.getState()
      if (room?.threadId === threadId) setRoom(null)
    },
    [threadId]
  )

  if (!canvas) return null
  const { frame } = canvas

  const begin = (
    event: React.PointerEvent<HTMLElement>,
    direction: ResizeDirection | null
  ) => {
    if (event.button !== 0) return
    gesture.current = {
      pointerId: event.pointerId,
      pointerType: event.pointerType,
      x: event.clientX,
      y: event.clientY,
      frame,
      direction,
      moved: false
    }
    event.currentTarget.setPointerCapture(event.pointerId)
    event.preventDefault()
    event.stopPropagation()
  }
  const follow = (event: React.PointerEvent<HTMLElement>) => {
    const current = gesture.current
    if (!current || current.pointerId !== event.pointerId) return
    const delta = { x: event.clientX - current.x, y: event.clientY - current.y }
    if (!current.moved && Math.hypot(delta.x, delta.y) < TAP_SLOP_PX) return
    current.moved = true
    const store = useMiniBrowserStore.getState()
    if (current.direction === null) {
      store.move(
        threadId,
        clampPosition(
          { x: current.frame.x + delta.x, y: current.frame.y + delta.y },
          container,
          current.frame
        )
      )
      return
    }
    const next = resizeFrame({
      start: current.frame,
      direction: current.direction,
      delta,
      source,
      container
    })
    store.resize(threadId, next.width, { x: next.x, y: next.y })
  }
  const end = (event: React.PointerEvent<HTMLElement>) => {
    const current = gesture.current
    if (current?.pointerId !== event.pointerId) return
    gesture.current = null
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId)
    }
    if (
      event.type === 'pointerup' &&
      current.pointerType === 'touch' &&
      current.direction === null &&
      !current.moved
    ) {
      setPillOpen((open) => !open)
    }
  }
  const openInPanel = () => {
    useMiniBrowserStore.getState().close(threadId)
    usePanelStore.getState().openSurface(threadId, 'browser')
  }

  return (
    <section
      aria-label={m.mini_browser()}
      className='pointer-events-none absolute z-20 select-none'
      style={{
        left: frame.x,
        top: frame.y,
        width: frame.width,
        height: frame.height
      }}
    >
      <div className='bg-muted pointer-events-auto absolute inset-0 overflow-hidden rounded-xl shadow-2xl'>
        <BrowserPage threadId={threadId} className='size-full' />
      </div>
      <div className='ring-border pointer-events-none absolute inset-0 rounded-xl ring-1 ring-inset' />
      <div
        ref={handle}
        data-pill-open={pillOpen ? '' : undefined}
        className='group pointer-events-auto absolute size-3 cursor-grab touch-none active:cursor-grabbing'
        style={{ right: PILL_INSET, top: PILL_INSET }}
        onPointerDown={(event) => begin(event, null)}
        onPointerMove={follow}
        onPointerUp={end}
        onPointerCancel={end}
      >
        <span
          className={cn(
            'ring-background/70 absolute top-0 right-0 block size-2 rounded-full shadow-sm ring-1 transition-opacity group-focus-within:opacity-0 group-hover:opacity-0 group-data-pill-open:opacity-0',
            agentActive ? 'bg-primary animate-pulse' : 'bg-foreground/25'
          )}
        />
        <div className='border-border bg-popover/90 pointer-events-none absolute top-0 right-0 flex cursor-grab items-center gap-0.5 rounded-lg border p-0.5 opacity-0 shadow-lg backdrop-blur-xl transition-opacity group-focus-within:pointer-events-auto group-focus-within:opacity-100 group-hover:pointer-events-auto group-hover:opacity-100 group-data-pill-open:pointer-events-auto group-data-pill-open:opacity-100 active:cursor-grabbing'>
          <span
            className='contents'
            onPointerDown={(event) => event.stopPropagation()}
          >
            <IconButton
              label={m.mini_browser_open_in_panel()}
              side='top'
              onClick={openInPanel}
            >
              <HugeiconsIcon icon={SidebarRightIcon} strokeWidth={2} />
            </IconButton>
            <IconButton
              label={m.mini_browser_close()}
              side='top'
              onClick={() => useMiniBrowserStore.getState().close(threadId)}
            >
              <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} />
            </IconButton>
          </span>
        </div>
      </div>
      {RESIZE_HANDLES.map(({ direction, className }) => (
        <div
          key={direction}
          role='presentation'
          className={cn('pointer-events-auto absolute touch-none', className)}
          onPointerDown={(event) => begin(event, direction)}
          onPointerMove={follow}
          onPointerUp={end}
          onPointerCancel={end}
        />
      ))}
    </section>
  )
}
