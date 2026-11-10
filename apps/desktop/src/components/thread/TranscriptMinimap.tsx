import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import { ArrowDown01Icon, ArrowUp01Icon } from '@hugeicons/core-free-icons'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'
import type { TranscriptRow } from '@/lib/tool-calls'
import {
  type MouseEvent,
  type RefObject,
  useEffect,
  useMemo,
  useState
} from 'react'
import {
  MINIMAP_MIN_ITEMS,
  MINIMAP_SPACING_PX,
  minimapCurrentIndex,
  minimapIndexAt,
  minimapItems,
  minimapTopPercent,
  plainPreview
} from '@/lib/transcript-minimap'

/** The map needs this much room beside the chat, so it never covers the messages. */
const MIN_GUTTER_PX = 48

/** How much a mark shrinks with its distance from the one pointed at. */
const FISHEYE = ['scale-x-100', 'scale-x-[0.667]', 'scale-x-[0.417]']

/**
 * A mark per message the user sent, beside the transcript. Marks on screen are brighter;
 * pointing at one previews it, clicking it scrolls there.
 */
export function TranscriptMinimap({
  rows,
  scrollRef,
  listRef,
  onJump
}: {
  rows: TranscriptRow[]
  scrollRef: RefObject<HTMLDivElement | null>
  listRef: RefObject<HTMLElement | null>
  /** Scrolls the transcript to the message with this id. */
  onJump: (id: string) => void
}) {
  const items = useMemo(() => minimapItems(rows), [rows])
  const idsKey = items.map((item) => item.id).join(' ')
  const [active, setActive] = useState<number | null>(null)
  const [current, setCurrent] = useState<number | null>(null)
  const [inView, setInView] = useState<boolean[]>([])
  const [fits, setFits] = useState(false)

  useEffect(() => {
    const scroll = scrollRef.current
    const list = listRef.current
    const ids = idsKey.split(' ')
    if (!scroll || !list || ids.length < MINIMAP_MIN_ITEMS) return
    let elements: (Element | null)[] = []
    /** Each message's top and bottom within the scrolled content. */
    let bounds: { top: number; bottom: number }[] = []
    let viewHeight = 0
    let stale = true
    let frame: number | null = null
    const find = () => {
      elements = ids.map((id) =>
        list.querySelector(`[data-item-id="${CSS.escape(id)}"]`)
      )
    }
    const layout = () => {
      if (elements.some((element) => !element?.isConnected)) find()
      const origin = scroll.getBoundingClientRect().top - scroll.scrollTop
      bounds = elements.map((element) => {
        const rect = element?.getBoundingClientRect()
        return rect
          ? { top: rect.top - origin, bottom: rect.bottom - origin }
          : { top: Number.NaN, bottom: Number.NaN }
      })
      viewHeight = scroll.clientHeight
      setFits((scroll.clientWidth - list.offsetWidth) / 2 >= MIN_GUTTER_PX)
      stale = false
    }
    const measure = () => {
      frame = null
      if (stale) layout()
      const top = scroll.scrollTop
      const bottom = top + viewHeight
      const seen = bounds.map(
        (bound) => bound.bottom > top && bound.top < bottom
      )
      setInView((previous) => (sameFlags(previous, seen) ? previous : seen))
      setCurrent(minimapCurrentIndex(bounds, top, bottom))
    }
    const schedule = () => {
      if (frame === null) frame = requestAnimationFrame(measure)
    }
    const relayout = () => {
      stale = true
      schedule()
    }
    find()
    schedule()
    scroll.addEventListener('scroll', schedule, { passive: true })
    const observer = new ResizeObserver(relayout)
    observer.observe(scroll)
    observer.observe(list)
    return () => {
      if (frame !== null) cancelAnimationFrame(frame)
      scroll.removeEventListener('scroll', schedule)
      observer.disconnect()
    }
  }, [idsKey, scrollRef, listRef])

  if (items.length < MINIMAP_MIN_ITEMS) return null

  const shown = active !== null && active < items.length ? active : null
  const preview = shown === null ? null : items[shown]
  const previous = current === null ? undefined : items[current - 1]
  const next = current === null ? undefined : items[current + 1]

  const indexAt = (event: MouseEvent<HTMLElement>) => {
    const rect = event.currentTarget.getBoundingClientRect()
    return minimapIndexAt(event.clientY, rect.top, rect.height, items.length)
  }
  const move = (step: number) =>
    setActive((index) =>
      Math.max(0, Math.min(items.length - 1, (index ?? current ?? 0) + step))
    )

  return (
    <div
      onWheel={(event) => scrollRef.current?.scrollBy({ top: event.deltaY })}
      className={cn(
        'absolute inset-y-0 left-0 z-10 w-14',
        !fits && 'invisible'
      )}
    >
      <div
        className='absolute top-1/2 left-3 w-8 -translate-y-1/2'
        style={{
          height: `min(${(items.length - 1) * MINIMAP_SPACING_PX}px, calc(100% - 8rem))`
        }}
      >
        <StepButton
          direction='previous'
          disabled={!previous}
          onClick={() => previous && onJump(previous.id)}
        />
        <button
          type='button'
          aria-label={m.jump_to_message({
            message: plainPreview(preview?.userText ?? '', 120)
          })}
          onMouseMove={(event) => setActive(indexAt(event))}
          onMouseLeave={() => setActive(null)}
          onFocus={() => setActive((index) => index ?? current ?? 0)}
          onBlur={() => setActive(null)}
          onClick={(event) => {
            const index = indexAt(event)
            const item = index === null ? undefined : items[index]
            if (item) onJump(item.id)
          }}
          onKeyDown={(event) => {
            if (event.key === 'ArrowDown') move(1)
            else if (event.key === 'ArrowUp') move(-1)
            else if (event.key === 'Enter' || event.key === ' ') {
              if (preview) onJump(preview.id)
            } else return
            event.preventDefault()
          }}
          className='focus-visible:ring-ring absolute inset-0 cursor-pointer rounded-sm outline-none focus-visible:ring-2'
        >
          {items.map((item, index) => {
            const distance = shown === null ? null : Math.abs(index - shown)
            return (
              <span
                key={item.id}
                aria-hidden
                style={{ top: `${minimapTopPercent(index, items.length)}%` }}
                className={cn(
                  'pointer-events-none absolute left-0 h-0.5 w-6 origin-left -translate-y-1/2 rounded-full transition-[transform,background-color] duration-150',
                  distance === 0
                    ? 'bg-foreground/70'
                    : inView[index]
                      ? 'bg-foreground/45'
                      : 'bg-muted-foreground/25',
                  distance === null
                    ? 'scale-x-[0.333]'
                    : (FISHEYE[distance] ?? 'scale-x-[0.333]')
                )}
              />
            )
          })}
        </button>
        {preview && shown !== null ? (
          <div
            style={{
              top: `${minimapTopPercent(shown, items.length)}%`,
              transform: `translateY(${
                shown === 0
                  ? '0%'
                  : shown === items.length - 1
                    ? '-100%'
                    : '-50%'
              })`
            }}
            className='bg-popover text-popover-foreground ring-foreground/10 pointer-events-none absolute left-8 z-20 w-80 rounded-lg p-3 shadow-md ring-1'
          >
            <p className='truncate text-sm font-medium'>
              {plainPreview(preview.userText) || m.user_message()}
            </p>
            {preview.assistantText ? (
              <p className='text-muted-foreground mt-1 line-clamp-3 text-sm'>
                {plainPreview(preview.assistantText)}
              </p>
            ) : null}
          </div>
        ) : null}
        <StepButton
          direction='next'
          disabled={!next}
          onClick={() => next && onJump(next.id)}
        />
      </div>
    </div>
  )
}

function sameFlags(a: boolean[], b: boolean[]) {
  return a.length === b.length && a.every((flag, index) => flag === b[index])
}

/**
 * Jumps to the turn before or after the one being read. It sits just past the end of the
 * rail and only shows while the pointer is on it.
 */
function StepButton({
  direction,
  disabled,
  onClick
}: {
  direction: 'previous' | 'next'
  disabled: boolean
  onClick: () => void
}) {
  const previous = direction === 'previous'
  const label = previous ? m.previous_message() : m.next_message()
  return (
    <span
      className={cn(
        'absolute left-1 z-10 inline-flex -translate-x-1/2 opacity-0 transition-opacity duration-150 focus-within:opacity-100 hover:opacity-100',
        previous ? 'bottom-[calc(100%+2px)]' : 'top-[calc(100%+2px)]'
      )}
    >
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              variant='ghost'
              size='icon-xs'
              aria-label={label}
              disabled={disabled}
              onClick={onClick}
            />
          }
        >
          <HugeiconsIcon
            icon={previous ? ArrowUp01Icon : ArrowDown01Icon}
            strokeWidth={2}
          />
        </TooltipTrigger>
        <TooltipContent side={previous ? 'top' : 'bottom'}>
          {label}
        </TooltipContent>
      </Tooltip>
    </span>
  )
}
