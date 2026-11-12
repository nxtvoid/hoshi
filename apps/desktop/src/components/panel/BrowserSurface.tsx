import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { HugeiconsIcon } from '@hugeicons/react'
import { openUrl } from '@tauri-apps/plugin-opener'
import { fitInside } from '@/lib/browser-layout'
import { usePanelStore } from '@/state/panel-store'
import { useBrowserStore } from '@/state/browser-store'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { useMiniBrowserStore } from '@/state/mini-browser-store'
import { useContext, useEffect, useRef, useState } from 'react'
import { PanelShown } from './panel-shown'
import {
  BrowserPage,
  useAgentActive,
  useBrowserViewing
} from '@/components/browser/BrowserPage'
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
  PictureInPictureOnIcon,
  Refresh01Icon,
  Tick02Icon
} from '@hugeicons/core-free-icons'

/** Quiet time after the panel's last resize before the page follows it. */
const FIT_DELAY_MS = 150

const SIZES = [
  { label: m.browser_size_desktop, width: 1280, height: 800 },
  { label: m.browser_size_tablet, width: 768, height: 1024 },
  { label: m.browser_size_phone, width: 390, height: 844 }
]

/**
 * The thread's headless browser page, live as it paints: the agent's, which the user
 * can take over by clicking and typing, or one the user opens from the address bar.
 */
export function BrowserSurface({ threadId }: { threadId: string }) {
  const frame = useBrowserStore((state) => state.frames[threadId])
  const page = useBrowserStore((state) => state.pages[threadId])
  const [address, setAddress] = useState(page?.url ?? '')
  const [failed, setFailed] = useState(false)
  /** The page takes the panel's size, until a size is picked here or by the agent. */
  const fill = page?.fitted ?? true
  const [panel, setPanel] = useState({ width: 0, height: 0 })
  const viewport = useRef<HTMLDivElement>(null)
  const shown = useContext(PanelShown)
  const agentActive = useAgentActive(threadId)
  useBrowserViewing(threadId, shown)

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
    if (page?.url) setAddress(page.url)
  }, [page?.url])

  const run = (command: string, args: Record<string, unknown>) =>
    invoke(command, { threadId, ...args }).catch(() => setFailed(true))
  const open = () => {
    if (!address.trim()) return
    setFailed(false)
    void run('browser_open', { url: address.trim() })
  }

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
          label={m.browser_float()}
          disabled={!page}
          onClick={() => {
            useMiniBrowserStore.getState().open(threadId)
            usePanelStore.getState().close(threadId)
          }}
        >
          <HugeiconsIcon icon={PictureInPictureOnIcon} strokeWidth={2} />
        </IconButton>
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
          <BrowserPage
            threadId={threadId}
            className={cn(!fill && 'shrink-0 shadow-sm')}
            style={fill ? undefined : fitInside(panel, frame)}
            onFailed={() => setFailed(true)}
          />
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
