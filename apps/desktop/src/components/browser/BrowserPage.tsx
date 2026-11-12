import { cn } from '@hoshi/utils'
import { invoke } from '@tauri-apps/api/core'
import { useBrowserStore } from '@/state/browser-store'
import type { BrowserInput } from '@/bindings/BrowserInput'
import { useEffect, useRef, useState } from 'react'

/** How long the agent's last action stays marked on the page. */
const AGENT_SHOWN_MS = 4000

/** Keys sent as they are; printable ones go as text. */
const KEYS = new Set([
  'Enter',
  'Tab',
  'Escape',
  'Backspace',
  'Delete',
  'ArrowLeft',
  'ArrowUp',
  'ArrowRight',
  'ArrowDown',
  'Home',
  'End',
  'PageUp',
  'PageDown'
])

/** Each thread's last input to its page; the next one waits for it. */
const inputs = new Map<string, Promise<unknown>>()

/** How many views show each thread's page; it streams while any does. */
const viewers = new Map<string, number>()

type Wheel = { x: number; y: number; deltaX: number; deltaY: number }

/** Streams the thread's page while `active`, however many views show it. */
export function useBrowserViewing(threadId: string, active: boolean) {
  useEffect(() => {
    if (!active) return
    const count = viewers.get(threadId) ?? 0
    viewers.set(threadId, count + 1)
    if (count === 0) void invoke('browser_view', { threadId, viewing: true })
    return () => {
      const left = (viewers.get(threadId) ?? 1) - 1
      if (left > 0) {
        viewers.set(threadId, left)
        return
      }
      viewers.delete(threadId)
      void invoke('browser_view', { threadId, viewing: false })
    }
  }, [threadId, active])
}

/** Whether the agent acted on the thread's page in the last few seconds. */
export function useAgentActive(threadId: string) {
  const agentAt = useBrowserStore(
    (state) => state.pages[threadId]?.agentAt ?? null
  )
  const [now, setNow] = useState(Date.now())
  useEffect(() => {
    if (agentAt === null) return
    setNow(Date.now())
    const wait = agentAt + AGENT_SHOWN_MS - Date.now()
    if (wait <= 0) return
    const timer = setTimeout(() => setNow(Date.now()), wait)
    return () => clearTimeout(timer)
  }, [agentAt])
  return agentAt !== null && now - agentAt < AGENT_SHOWN_MS
}

/**
 * The thread's page as it last painted, which takes clicks, scrolls and keys. Nothing
 * until the page has a frame.
 */
export function BrowserPage({
  threadId,
  className,
  style,
  onFailed
}: {
  threadId: string
  className?: string
  style?: React.CSSProperties
  onFailed?: () => void
}) {
  const frame = useBrowserStore((state) => state.frames[threadId])
  const page = useBrowserStore((state) => state.pages[threadId])
  const agentActive = useAgentActive(threadId)
  const image = useRef<HTMLImageElement>(null)
  const wheel = useRef<Wheel | null>(null)
  const wheelFrame = useRef<number | null>(null)

  useEffect(
    () => () => {
      if (wheelFrame.current !== null) cancelAnimationFrame(wheelFrame.current)
      wheelFrame.current = null
      wheel.current = null
    },
    []
  )

  if (!page || !frame) return null

  const send = (input: BrowserInput) => {
    const sent = (inputs.get(threadId) ?? Promise.resolve()).then(() =>
      invoke('browser_input', { threadId, input }).catch(() => onFailed?.())
    )
    inputs.set(threadId, sent)
    void sent.then(() => {
      if (inputs.get(threadId) === sent) inputs.delete(threadId)
    })
  }
  const scroll = (
    point: { x: number; y: number },
    deltaX: number,
    deltaY: number
  ) => {
    const waiting = wheel.current
    wheel.current = {
      ...point,
      deltaX: (waiting?.deltaX ?? 0) + deltaX,
      deltaY: (waiting?.deltaY ?? 0) + deltaY
    }
    if (wheelFrame.current !== null) return
    wheelFrame.current = requestAnimationFrame(() => {
      wheelFrame.current = null
      const pending = wheel.current
      wheel.current = null
      if (pending) send({ kind: 'scroll', ...pending })
    })
  }
  /** A point of the shown frame, in the page's own pixels. */
  const pagePoint = (clientX: number, clientY: number) => {
    const rect = image.current?.getBoundingClientRect()
    if (!rect || rect.width === 0) return null
    const scale = frame.width / rect.width
    return {
      x: (clientX - rect.left) * scale,
      y: (clientY - rect.top) * scale
    }
  }
  const marker =
    agentActive && page.agentPoint
      ? {
          left: (page.agentPoint.x / frame.width) * 100,
          top: (page.agentPoint.y / frame.height) * 100
        }
      : null

  return (
    <div
      role='application'
      aria-label={page.title || page.url}
      // biome-ignore lint/a11y/noNoninteractiveTabindex: the page takes the keyboard
      tabIndex={0}
      className={cn(
        'focus-visible:ring-ring relative outline-none focus-visible:ring-2 focus-visible:ring-inset',
        className
      )}
      style={style}
      onKeyDown={(event) => {
        if (event.ctrlKey || event.metaKey || event.altKey) return
        if (KEYS.has(event.key)) send({ kind: 'key', key: event.key })
        else if (event.key.length === 1) {
          send({ kind: 'text', text: event.key })
        } else return
        event.preventDefault()
      }}
      onClick={(event) => {
        event.currentTarget.focus()
        const point = pagePoint(event.clientX, event.clientY)
        if (point) send({ kind: 'click', ...point })
      }}
      onWheel={(event) => {
        const point = pagePoint(event.clientX, event.clientY)
        if (point) scroll(point, event.deltaX, event.deltaY)
      }}
      onPaste={(event) => {
        const text = event.clipboardData.getData('text')
        if (text) send({ kind: 'text', text })
        event.preventDefault()
      }}
    >
      <img
        ref={image}
        src={frame.url}
        alt={page.title || page.url}
        draggable={false}
        className='block h-auto w-full select-none'
      />
      {marker ? (
        <span
          className='border-primary pointer-events-none absolute size-8 -translate-x-1/2 -translate-y-1/2 animate-pulse rounded-full border-2'
          style={{ left: `${marker.left}%`, top: `${marker.top}%` }}
        />
      ) : null}
    </div>
  )
}
