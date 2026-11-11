import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { HugeiconsIcon } from '@hugeicons/react'
import { openUrl } from '@tauri-apps/plugin-opener'
import { fitInside } from '@/lib/browser-layout'
import { useBrowserStore } from '@/state/browser-store'
import { IconButton } from '@hoshi/ui/components/icon-button'
import type { BrowserInput } from '@/bindings/BrowserInput'
import { useContext, useEffect, useRef, useState } from 'react'
import { PanelShown } from './panel-shown'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger
} from '@hoshi/ui/components/dropdown-menu'
import {
  ArrowLeft01Icon,
  ArrowRight01Icon,
  LinkSquare02Icon,
  Refresh01Icon,
  Tick02Icon
} from '@hugeicons/core-free-icons'

/** How long the agent's last action stays marked on the page. */
const AGENT_SHOWN_MS = 4000
/** Quiet time after the panel's last resize before the page follows it. */
const FIT_DELAY_MS = 150

const SIZES = [
  { label: m.browser_size_desktop, width: 1280, height: 800 },
  { label: m.browser_size_tablet, width: 768, height: 1024 },
  { label: m.browser_size_phone, width: 390, height: 844 }
]

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

type Wheel = { x: number; y: number; deltaX: number; deltaY: number }

/**
 * The thread's headless browser page, live as it paints: the agent's, which the user
 * can take over by clicking and typing, or one the user opens from the address bar.
 */
export function BrowserSurface({ threadId }: { threadId: string }) {
  const frame = useBrowserStore((state) => state.frames[threadId])
  const page = useBrowserStore((state) => state.pages[threadId])
  const [address, setAddress] = useState(page?.url ?? '')
  const [failed, setFailed] = useState(false)
  const [now, setNow] = useState(Date.now())
  /** The page takes the panel's size, until a size is picked here or by the agent. */
  const fill = page?.fitted ?? true
  const [panel, setPanel] = useState({ width: 0, height: 0 })
  const image = useRef<HTMLImageElement>(null)
  const viewport = useRef<HTMLDivElement>(null)
  const wheel = useRef<Wheel | null>(null)
  const wheelFrame = useRef<number | null>(null)
  const shown = useContext(PanelShown)

  useEffect(
    () => () => {
      if (wheelFrame.current !== null) cancelAnimationFrame(wheelFrame.current)
      wheelFrame.current = null
      wheel.current = null
    },
    []
  )

  useEffect(() => {
    const box = viewport.current
    if (!fill || !box) return
    let timer: ReturnType<typeof setTimeout> | undefined
    const fit = () => {
      clearTimeout(timer)
      timer = setTimeout(() => {
        const width = Math.floor(box.clientWidth)
        const height = Math.floor(box.clientHeight)
        if (width > 0 && height > 0) {
          void invoke('browser_fit', { threadId, width, height }).catch(
            () => {}
          )
        }
      }, FIT_DELAY_MS)
    }
    const observer = new ResizeObserver(fit)
    observer.observe(box)
    fit()
    return () => {
      clearTimeout(timer)
      observer.disconnect()
    }
  }, [fill, threadId])

  useEffect(() => {
    const box = viewport.current
    if (!box) return
    const measure = () =>
      setPanel({ width: box.clientWidth, height: box.clientHeight })
    const observer = new ResizeObserver(measure)
    observer.observe(box)
    measure()
    return () => observer.disconnect()
  }, [])

  useEffect(() => {
    if (!shown) return
    void invoke('browser_view', { threadId, viewing: true })
    return () => {
      void invoke('browser_view', { threadId, viewing: false })
    }
  }, [threadId, shown])

  useEffect(() => {
    if (page?.url) setAddress(page.url)
  }, [page?.url])

  const agentAt = page?.agentAt ?? null
  useEffect(() => {
    if (agentAt === null) return
    const wait = agentAt + AGENT_SHOWN_MS - Date.now()
    if (wait <= 0) return
    const timer = setTimeout(() => setNow(Date.now()), wait)
    return () => clearTimeout(timer)
  }, [agentAt])
  const agentActive = agentAt !== null && now - agentAt < AGENT_SHOWN_MS

  const run = (command: string, args: Record<string, unknown>) =>
    invoke(command, { threadId, ...args }).catch(() => setFailed(true))
  const send = (input: BrowserInput) => {
    const sent = (inputs.get(threadId) ?? Promise.resolve()).then(() =>
      run('browser_input', { input })
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
  const open = () => {
    if (!address.trim()) return
    setFailed(false)
    void run('browser_open', { url: address.trim() })
  }

  /** A point of the shown frame, in the page's own pixels. */
  const pagePoint = (clientX: number, clientY: number) => {
    const rect = image.current?.getBoundingClientRect()
    if (!rect || !frame || rect.width === 0) return null
    const scale = frame.width / rect.width
    return {
      x: (clientX - rect.left) * scale,
      y: (clientY - rect.top) * scale
    }
  }
  const marker =
    agentActive && page?.agentPoint && frame
      ? {
          left: (page.agentPoint.x / frame.width) * 100,
          top: (page.agentPoint.y / frame.height) * 100
        }
      : null

  return (
    <div className='flex min-h-0 flex-1 flex-col'>
      <div className='border-border flex shrink-0 items-center gap-1 border-b px-2 py-1.5'>
        <IconButton
          label={m.browser_back()}
          disabled={!page}
          onClick={() => void run('browser_go', { delta: -1 })}
        >
          <HugeiconsIcon icon={ArrowLeft01Icon} strokeWidth={2} />
        </IconButton>
        <IconButton
          label={m.browser_forward()}
          disabled={!page}
          onClick={() => void run('browser_go', { delta: 1 })}
        >
          <HugeiconsIcon icon={ArrowRight01Icon} strokeWidth={2} />
        </IconButton>
        <IconButton
          label={m.browser_reload()}
          disabled={!page}
          onClick={() => void run('browser_go', { delta: 0 })}
        >
          <HugeiconsIcon icon={Refresh01Icon} strokeWidth={2} />
        </IconButton>
        <form
          className='min-w-0 flex-1'
          onSubmit={(event) => {
            event.preventDefault()
            open()
          }}
        >
          <input
            value={address}
            onChange={(event) => setAddress(event.target.value)}
            onFocus={(event) => event.currentTarget.select()}
            aria-label={m.browser_address()}
            placeholder={m.browser_address_placeholder()}
            spellCheck={false}
            className='border-border bg-background focus-visible:ring-ring h-7 w-full rounded-md border px-2 font-mono text-xs outline-none focus-visible:ring-2'
          />
        </form>
        <DropdownMenu>
          <DropdownMenuTrigger
            disabled={!page}
            className='border-border text-muted-foreground hover:text-foreground h-7 shrink-0 rounded-md border px-2 text-xs tabular-nums disabled:opacity-50'
          >
            {fill
              ? m.browser_size_fill()
              : frame
                ? `${frame.width} × ${frame.height}`
                : m.browser_size()}
          </DropdownMenuTrigger>
          <DropdownMenuContent align='end' className='min-w-52'>
            <DropdownMenuItem
              onClick={() => {
                const box = viewport.current
                if (!box) return
                void run('browser_fit', {
                  width: Math.floor(box.clientWidth),
                  height: Math.floor(box.clientHeight)
                })
              }}
            >
              <span className='flex-1 whitespace-nowrap'>
                {m.browser_size_fill()}
              </span>
              {fill ? (
                <HugeiconsIcon icon={Tick02Icon} strokeWidth={2} />
              ) : null}
            </DropdownMenuItem>
            {SIZES.map((size) => (
              <DropdownMenuItem
                key={size.width}
                onClick={() => {
                  void run('browser_resize', {
                    width: size.width,
                    height: size.height
                  })
                }}
              >
                <span className='flex-1 whitespace-nowrap'>{size.label()}</span>
                <span className='text-muted-foreground text-xs whitespace-nowrap tabular-nums'>
                  {size.width} × {size.height}
                </span>
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
        <IconButton
          label={m.browser_open_external()}
          disabled={!page?.url.startsWith('http')}
          onClick={() => {
            if (page?.url) void openUrl(page.url)
          }}
        >
          <HugeiconsIcon icon={LinkSquare02Icon} strokeWidth={2} />
        </IconButton>
      </div>
      <div
        ref={viewport}
        className={cn(
          'relative min-h-0 flex-1',
          fill
            ? 'bg-background overflow-hidden'
            : 'bg-muted/30 flex items-center justify-center overflow-hidden'
        )}
      >
        {page && frame ? (
          <div
            role='application'
            aria-label={page.title || page.url}
            // biome-ignore lint/a11y/noNoninteractiveTabindex: the page takes the keyboard
            tabIndex={0}
            className={cn(
              'focus-visible:ring-ring relative outline-none focus-visible:ring-2 focus-visible:ring-inset',
              !fill && 'shrink-0 shadow-sm'
            )}
            style={fill ? undefined : fitInside(panel, frame)}
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
              src={`data:image/jpeg;base64,${frame.data}`}
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
        ) : (
          <div className='text-muted-foreground flex h-full flex-col items-center justify-center gap-1 px-6 text-center text-xs'>
            <p className='text-foreground text-sm'>
              {failed ? m.browser_failed() : m.browser_empty()}
            </p>
            <p>{m.browser_empty_hint()}</p>
          </div>
        )}
        {agentActive ? (
          <span className='bg-popover border-border text-muted-foreground absolute right-3 bottom-3 flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs shadow-sm'>
            <span className='bg-primary size-1.5 animate-pulse rounded-full' />
            {m.browser_agent_using()}
          </span>
        ) : null}
      </div>
      {page && page.errors > 0 ? (
        <div
          className={cn(
            'border-border text-status-error shrink-0 border-t px-3 py-1.5 text-xs'
          )}
        >
          {m.browser_console_errors({ count: page.errors })}
        </div>
      ) : null}
    </div>
  )
}
