import { m } from '@/paraglide/messages'
import { Input } from '@hoshi/ui/components/input'
import {
  type PointerEvent as ReactPointerEvent,
  useEffect,
  useId,
  useRef,
  useState
} from 'react'
import {
  Popover,
  PopoverContent,
  PopoverTrigger
} from '@hoshi/ui/components/popover'
import {
  BLACK,
  colorToHex,
  colorToRgb,
  type Hsv,
  hsvToRgb,
  isColor,
  parseColor,
  rgbToHex,
  rgbToHsv
} from '@/lib/color'

const HEX = /^#[0-9a-f]{6}$/i

function alphaSuffix(value: string): string {
  const hex = colorToHex(value)
  return hex && hex.length === 9 ? hex.slice(7) : ''
}

function opaqueHex(value: string): string {
  return (colorToHex(value) ?? '#000000').slice(0, 7)
}

const hsvToHex = (hsv: Hsv) => rgbToHex(hsvToRgb(hsv))
const hexToHsv = (hex: string) => rgbToHsv(colorToRgb(hex, BLACK))

function rgbText(hex: string): string {
  const { r, g, b } = colorToRgb(hex, BLACK)
  return `${r}, ${g}, ${b}`
}

function rgbTextToHex(text: string): string | null {
  const parts = text.split(/[\s,]+/).filter(Boolean)
  if (parts.length !== 3) return null
  const channels = parts.map((part) => Number(part))
  if (
    channels.some(
      (value) => !Number.isInteger(value) || value < 0 || value > 255
    )
  ) {
    return null
  }
  return rgbToHex({ r: channels[0]!, g: channels[1]!, b: channels[2]! })
}

/** Batches a drag's changes to one per frame, so the live preview keeps up. */
function useFrameCommit(onChange: (value: string) => void) {
  const pending = useRef<string | null>(null)
  const frame = useRef<number | null>(null)
  const latest = useRef(onChange)
  latest.current = onChange
  const flush = () => {
    if (frame.current !== null) cancelAnimationFrame(frame.current)
    frame.current = null
    const value = pending.current
    pending.current = null
    if (value !== null) latest.current(value)
  }
  const schedule = (value: string) => {
    pending.current = value
    frame.current ??= requestAnimationFrame(flush)
  }
  useEffect(() => () => flush(), [])
  return { schedule, flush }
}

function dragWithin(
  event: ReactPointerEvent<HTMLElement>,
  onMove: (x: number, y: number) => void,
  onEnd: () => void
) {
  const element = event.currentTarget
  const rect = element.getBoundingClientRect()
  const move = (clientX: number, clientY: number) =>
    onMove(
      Math.min(1, Math.max(0, (clientX - rect.left) / rect.width)),
      Math.min(1, Math.max(0, (clientY - rect.top) / rect.height))
    )
  element.setPointerCapture(event.pointerId)
  move(event.clientX, event.clientY)
  const handleMove = (next: PointerEvent) => move(next.clientX, next.clientY)
  const handleUp = () => {
    element.removeEventListener('pointermove', handleMove)
    element.removeEventListener('pointerup', handleUp)
    element.removeEventListener('pointercancel', handleUp)
    onEnd()
  }
  element.addEventListener('pointermove', handleMove)
  element.addEventListener('pointerup', handleUp)
  element.addEventListener('pointercancel', handleUp)
}

function ColorPickerPanel({
  label,
  value,
  onChange
}: {
  label: string
  value: string
  onChange: (value: string) => void
}) {
  const hex = opaqueHex(value)
  const suffix = alphaSuffix(value)
  const [hsv, setHsv] = useState(() => hexToHsv(hex))
  const [hexDraft, setHexDraft] = useState(hex)
  const [rgbDraft, setRgbDraft] = useState(() => rgbText(hex))
  const [typing, setTyping] = useState(false)
  const [synced, setSynced] = useState(hex)
  const current = hsvToHex(hsv)
  const commit = useFrameCommit(onChange)
  const id = useId()

  if (synced !== hex) {
    setSynced(hex)
    if (!typing) {
      setHexDraft(hex)
      setRgbDraft(rgbText(hex))
    }
    if (current !== hex) setHsv(hexToHsv(hex))
  }

  const commitHsv = (next: Hsv) => {
    setHsv(next)
    const nextHex = hsvToHex(next)
    setHexDraft(nextHex)
    setRgbDraft(rgbText(nextHex))
    commit.schedule(nextHex + suffix)
  }

  const resetDrafts = () => {
    setTyping(false)
    setHexDraft(current)
    setRgbDraft(rgbText(current))
  }

  return (
    <div className='flex flex-col gap-3'>
      <div className='flex items-center justify-between gap-2'>
        <div className='min-w-0'>
          <p className='truncate text-xs font-semibold'>{label}</p>
          <p className='text-muted-foreground text-xs'>{m.choose_a_color()}</p>
        </div>
        <span
          className='ring-foreground/15 size-7 shrink-0 rounded-full ring-1'
          style={{ backgroundColor: current }}
        />
      </div>
      <div
        role='slider'
        tabIndex={0}
        aria-label={m.color_saturation_brightness({ name: label })}
        aria-valuetext={current}
        aria-valuenow={Math.round(hsv.s * 100)}
        className='focus-visible:ring-ring relative h-36 cursor-crosshair touch-none rounded-md outline-none focus-visible:ring-2'
        style={{
          backgroundColor: hsvToHex({ h: hsv.h, s: 1, v: 1 }),
          backgroundImage:
            'linear-gradient(to top, oklch(0 0 0), transparent), linear-gradient(to right, oklch(1 0 0), transparent)'
        }}
        onPointerDown={(event) =>
          dragWithin(
            event,
            (x, y) => commitHsv({ h: hsv.h, s: x, v: 1 - y }),
            commit.flush
          )
        }
        onKeyDown={(event) => {
          const step = event.shiftKey ? 0.1 : 0.02
          const moves: Record<string, Partial<Hsv>> = {
            ArrowLeft: { s: Math.max(0, hsv.s - step) },
            ArrowRight: { s: Math.min(1, hsv.s + step) },
            ArrowDown: { v: Math.max(0, hsv.v - step) },
            ArrowUp: { v: Math.min(1, hsv.v + step) }
          }
          const move = moves[event.key]
          if (!move) return
          event.preventDefault()
          commitHsv({ ...hsv, ...move })
          commit.flush()
        }}
      >
        <span
          className='border-background pointer-events-none absolute size-3.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 shadow-sm'
          style={{ left: `${hsv.s * 100}%`, top: `${(1 - hsv.v) * 100}%` }}
        />
      </div>
      <div
        role='slider'
        tabIndex={0}
        aria-label={m.color_hue({ name: label })}
        aria-valuemin={0}
        aria-valuemax={360}
        aria-valuenow={Math.round(hsv.h)}
        className='focus-visible:ring-ring relative h-3 cursor-pointer touch-none rounded-full outline-none focus-visible:ring-2'
        style={{
          backgroundImage: `linear-gradient(to right, ${[
            0, 60, 120, 180, 240, 300, 360
          ]
            .map((h) => hsvToHex({ h, s: 1, v: 1 }))
            .join(', ')})`
        }}
        onPointerDown={(event) =>
          dragWithin(
            event,
            (x) => commitHsv({ ...hsv, h: Math.min(359.9, x * 360) }),
            commit.flush
          )
        }
        onKeyDown={(event) => {
          const step = event.shiftKey ? 15 : 3
          if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return
          event.preventDefault()
          const delta = event.key === 'ArrowLeft' ? -step : step
          commitHsv({ ...hsv, h: (hsv.h + delta + 360) % 360 })
          commit.flush()
        }}
      >
        <span
          className='border-background pointer-events-none absolute top-1/2 size-3.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 shadow-sm'
          style={{
            left: `${(hsv.h / 360) * 100}%`,
            backgroundColor: hsvToHex({ h: hsv.h, s: 1, v: 1 })
          }}
        />
      </div>
      <div className='grid grid-cols-[1fr_1.2fr] gap-2'>
        <div className='flex min-w-0 flex-col gap-1'>
          <label
            htmlFor={`${id}-hex`}
            className='text-muted-foreground px-1 text-[0.625rem] font-semibold tracking-widest uppercase'
          >
            {m.color_hex()}
          </label>
          <Input
            id={`${id}-hex`}
            aria-label={m.color_hex_value({ name: label })}
            className='h-7 font-mono text-xs md:text-xs'
            spellCheck={false}
            value={hexDraft}
            onFocus={() => setTyping(true)}
            onBlur={resetDrafts}
            onChange={(event) => {
              const next = event.currentTarget.value
              setHexDraft(next)
              if (!HEX.test(next)) return
              setHsv(hexToHsv(next))
              setRgbDraft(rgbText(next))
              onChange(next.toLowerCase() + suffix)
            }}
          />
        </div>
        <div className='flex min-w-0 flex-col gap-1'>
          <label
            htmlFor={`${id}-rgb`}
            className='text-muted-foreground px-1 text-[0.625rem] font-semibold tracking-widest uppercase'
          >
            {m.color_rgb()}
          </label>
          <Input
            id={`${id}-rgb`}
            aria-label={m.color_rgb_value({ name: label })}
            className='h-7 font-mono text-xs md:text-xs'
            spellCheck={false}
            value={rgbDraft}
            onFocus={() => setTyping(true)}
            onBlur={resetDrafts}
            onChange={(event) => {
              const next = event.currentTarget.value
              setRgbDraft(next)
              const nextHex = rgbTextToHex(next)
              if (!nextHex) return
              setHsv(hexToHsv(nextHex))
              setHexDraft(nextHex)
              onChange(nextHex + suffix)
            }}
          />
        </div>
      </div>
    </div>
  )
}

/** One editable color: a swatch that opens a picker, and its value as text. */
export function ThemeColorField({
  label,
  value,
  onChange,
  onEditStart,
  onEditCancel
}: {
  label: string
  value: string
  onChange: (value: string) => void
  onEditStart: () => void
  onEditCancel: () => void
}) {
  const valid = isColor(value)
  const shown = value.trim().toLowerCase().startsWith('oklch(')
    ? (colorToHex(value) ?? value)
    : value
  const cancelled = useRef(false)

  return (
    <div className='flex min-h-10 min-w-0 items-center gap-2 rounded-lg px-2 py-1'>
      <span className='text-muted-foreground min-w-0 flex-1 truncate text-sm'>
        {label}
      </span>
      <Popover
        onOpenChange={(open, details) => {
          cancelled.current = !open && details.reason === 'escape-key'
          if (open) onEditStart()
          else if (cancelled.current) onEditCancel()
        }}
      >
        <PopoverTrigger
          aria-label={m.choose_color_named({ name: label })}
          className='ring-foreground/30 focus-visible:outline-ring relative size-6 shrink-0 cursor-pointer rounded-full ring-1 transition-transform hover:scale-105 focus-visible:outline-2'
          style={{ backgroundColor: valid ? value : undefined }}
        />
        <PopoverContent align='end' className='w-72 p-3'>
          <ColorPickerPanel
            label={label}
            value={valid ? value : '#000000'}
            onChange={(next) => {
              if (!cancelled.current) onChange(next)
            }}
          />
        </PopoverContent>
      </Popover>
      <Input
        aria-invalid={!valid}
        aria-label={m.color_hex_value({ name: label })}
        className='h-7 w-28 shrink-0 font-mono text-xs md:text-xs'
        spellCheck={false}
        value={shown}
        onChange={(event) => onChange(event.currentTarget.value)}
      />
    </div>
  )
}

export function previewColor(value: string, fallback: string): string {
  return parseColor(value) ? value : fallback
}
