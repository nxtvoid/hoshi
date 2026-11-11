import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { convertFileSrc } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'
import { usePageTheme } from './use-page-theme'
import { useWholeToolCall } from './use-whole-tool-call'
import type { ToolCallItem } from '@/lib/tool-calls'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { memo, useEffect, useLayoutEffect, useRef, useState } from 'react'
import {
  Cancel01Icon,
  Maximize01Icon,
  MinusSignIcon,
  PlusSignIcon
} from '@hugeicons/core-free-icons'
import { Dialog, DialogContent, DialogTitle } from '@hoshi/ui/components/dialog'
import {
  frameHeight,
  type HtmlPage,
  PAGE_SCHEME,
  readHtmlPage,
  readPageMessage,
  themeFragment,
  themeMessage
} from '@/lib/html-page'

const COLUMN_WIDTH = 768
const ZOOMS = [0.5, 0.75, 1, 1.25, 1.5, 2]

/** A page the agent showed with `html_render`, on the thread's background. */
export const HtmlPageRow = memo(function HtmlPageRow({
  item
}: {
  item: ToolCallItem
}) {
  const whole = useWholeToolCall(item, true)
  const page = readHtmlPage(whole.output)
  return page ? <HtmlPageFrame page={page} /> : null
})

function HtmlPageFrame({ page }: { page: HtmlPage }) {
  const container = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState(COLUMN_WIDTH)
  const [contentHeight, setContentHeight] = useState<number | null>(null)
  const [expanded, setExpanded] = useState(false)

  useLayoutEffect(() => {
    const element = container.current
    if (!element) return
    const observer = new ResizeObserver(([entry]) => {
      if (entry) setWidth(Math.round(entry.contentRect.width))
    })
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  return (
    <div ref={container} className='group/page relative -mx-2'>
      <PageDocument
        page={page}
        onHeight={setContentHeight}
        style={{ height: frameHeight(page, width, contentHeight) }}
        className='w-full'
      />
      <IconButton
        label={m.html_page_expand()}
        onClick={() => setExpanded(true)}
        className='bg-background/80 absolute top-2 right-2 opacity-0 group-hover/page:opacity-100 focus-visible:opacity-100'
      >
        <HugeiconsIcon icon={Maximize01Icon} strokeWidth={2} />
      </IconButton>
      {expanded ? (
        <ExpandedPage page={page} onClose={() => setExpanded(false)} />
      ) : null}
    </div>
  )
}

function ExpandedPage({
  page,
  onClose
}: {
  page: HtmlPage
  onClose: () => void
}) {
  const [zoom, setZoom] = useState(1)
  const step = (by: number) => {
    const next = ZOOMS[ZOOMS.indexOf(zoom) + by]
    if (next !== undefined) setZoom(next)
  }

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent
        showCloseButton={false}
        className='flex h-[88vh] w-[92vw] max-w-none flex-col gap-0 overflow-hidden p-0 sm:max-w-none'
      >
        <div className='border-border flex items-center gap-1 border-b py-1.5 pr-1.5 pl-4'>
          <DialogTitle className='min-w-0 flex-1 truncate text-sm font-medium'>
            {page.title}
          </DialogTitle>
          <IconButton
            label={m.html_page_zoom_out()}
            disabled={zoom === ZOOMS[0]}
            onClick={() => step(-1)}
          >
            <HugeiconsIcon icon={MinusSignIcon} strokeWidth={2} />
          </IconButton>
          <button
            type='button'
            aria-label={m.html_page_zoom_reset()}
            onClick={() => setZoom(1)}
            className='text-muted-foreground hover:text-foreground w-12 text-center text-xs tabular-nums'
          >
            {Math.round(zoom * 100)}%
          </button>
          <IconButton
            label={m.html_page_zoom_in()}
            disabled={zoom === ZOOMS.at(-1)}
            onClick={() => step(1)}
          >
            <HugeiconsIcon icon={PlusSignIcon} strokeWidth={2} />
          </IconButton>
          <IconButton label={m.html_page_close()} onClick={onClose}>
            <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} />
          </IconButton>
        </div>
        <div className='min-h-0 flex-1 overflow-hidden'>
          <PageDocument
            page={page}
            style={{
              zoom,
              width: `${100 / zoom}%`,
              height: `${100 / zoom}%`
            }}
          />
        </div>
      </DialogContent>
    </Dialog>
  )
}

/**
 * The page in a sandboxed frame of its own origin. It starts in the app's theme, follows
 * it, reports its height, and opens links it was clicked on in the browser.
 */
function PageDocument({
  page,
  onHeight,
  style,
  className
}: {
  page: HtmlPage
  onHeight?: (height: number) => void
  style: React.CSSProperties
  className?: string
}) {
  const frame = useRef<HTMLIFrameElement>(null)
  const theme = usePageTheme()
  const [src] = useState(
    () => convertFileSrc(page.id, PAGE_SCHEME) + themeFragment(theme)
  )

  useEffect(() => {
    frame.current?.contentWindow?.postMessage(themeMessage(theme), '*')
  }, [theme])

  useEffect(() => {
    const listen = (event: MessageEvent) => {
      const element = frame.current
      if (!element || event.source !== element.contentWindow) return
      const message = readPageMessage(event.data)
      if (message?.kind === 'height') onHeight?.(message.height)
      if (
        message?.kind === 'link' &&
        document.activeElement === element &&
        navigator.userActivation?.isActive !== false
      ) {
        void openUrl(message.url)
      }
    }
    window.addEventListener('message', listen)
    return () => window.removeEventListener('message', listen)
  }, [onHeight])

  return (
    <iframe
      ref={frame}
      src={src}
      title={page.title}
      sandbox='allow-scripts allow-forms'
      loading='lazy'
      style={style}
      className={cn('block border-0 bg-transparent', className)}
    />
  )
}
