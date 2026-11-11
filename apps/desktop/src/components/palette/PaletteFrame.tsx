import { cn, offsets } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { Kbd } from '@hoshi/ui/components/kbd'
import { HugeiconsIcon } from '@hugeicons/react'
import { isMac } from '@/components/brand/EditorIcon'
import { type ReactNode, useEffect, useRef, useState } from 'react'
import { ArrowLeft01Icon, Search01Icon } from '@hugeicons/core-free-icons'

export const PALETTE_ICON = 'text-muted-foreground size-4 shrink-0'

export type PaletteEntry = {
  key: string
  /** The palette closes after it, or once its promise resolves to `true`; `false` keeps it open to show why. */
  run: () => unknown
  content: ReactNode
  disabled?: boolean
}
/** An empty label shows no heading; `divider` draws a line under the group. */
export type PaletteGroup = {
  label: string
  entries: PaletteEntry[]
  divider?: boolean
}

const hasModifier = (event: { metaKey: boolean; ctrlKey: boolean }) =>
  isMac() ? event.metaKey : event.ctrlKey

/** The search field, the grouped results driven from it, and the key hints. */
export function PaletteFrame({
  query,
  onQueryChange,
  placeholder,
  groups,
  empty,
  status,
  children,
  onClose,
  onBack,
  onSubmit,
  onTab,
  accessory,
  autoHighlight = true,
  enterLabel,
  footer,
  onActiveChange
}: {
  query: string
  onQueryChange: (query: string) => void
  placeholder: string
  groups: PaletteGroup[]
  /** Shown when no group has entries. */
  empty: ReactNode
  /** A line under the field, such as an error. */
  status?: ReactNode
  /** Above the results, such as what a step is about to act on. */
  children?: ReactNode
  onClose: () => void
  /** Shows a back arrow, also reached with Backspace in an empty field. */
  onBack?: () => void
  /** Enter with no entry highlighted, or with the platform's modifier held. */
  onSubmit?: () => void
  /** Tab, with the highlighted entry if any. */
  onTab?: (entry: PaletteEntry | undefined) => void
  /** At the end of the field, such as a submit button. */
  accessory?: ReactNode
  /** Whether the first entry is highlighted before the arrows move. */
  autoHighlight?: boolean
  enterLabel?: string
  /** At the end of the key hints. */
  footer?: ReactNode
  /** The highlighted entry's key, each time it changes. */
  onActiveChange?: (key: string | undefined) => void
}) {
  const initial = autoHighlight ? 0 : -1
  const [active, setActive] = useState(initial)
  const [busy, setBusy] = useState<string | null>(null)
  const list = useRef<HTMLDivElement>(null)
  const entries = groups.flatMap((group) => group.entries)
  const current = Math.min(active, entries.length - 1)
  const activeKey = entries[current]?.key

  useEffect(() => setActive(initial), [query, initial])

  useEffect(() => onActiveChange?.(activeKey), [activeKey])

  useEffect(() => {
    list.current
      ?.querySelector(`[data-index="${current}"]`)
      ?.scrollIntoView({ block: 'nearest' })
  }, [current])

  const run = async (entry: PaletteEntry | undefined) => {
    if (!entry || entry.disabled || busy) return
    const result = entry.run()
    if (!(result instanceof Promise)) return onClose()
    setBusy(entry.key)
    const close = await result
    setBusy(null)
    if (close) onClose()
  }

  const move = (step: 1 | -1) => {
    if (entries.length === 0) return
    if (current < 0) return setActive(step === 1 ? 0 : entries.length - 1)
    setActive((current + step + entries.length) % entries.length)
  }

  const firstIndex = offsets(groups.map((group) => group.entries.length))
  return (
    <>
      <label className='border-border flex h-12 items-center gap-2.5 border-b px-4'>
        {onBack ? (
          <button
            type='button'
            onClick={onBack}
            aria-label={m.palette_back()}
            className='text-muted-foreground hover:text-foreground -ml-1 flex size-6 shrink-0 cursor-pointer items-center justify-center rounded-md'
          >
            <HugeiconsIcon
              icon={ArrowLeft01Icon}
              strokeWidth={2}
              className='size-4'
            />
          </button>
        ) : (
          <HugeiconsIcon
            icon={Search01Icon}
            strokeWidth={2}
            className={PALETTE_ICON}
          />
        )}
        <input
          // biome-ignore lint/a11y/noAutofocus: the palette exists to be typed into
          autoFocus
          value={query}
          spellCheck={false}
          onChange={(event) => onQueryChange(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.nativeEvent.isComposing) return
            if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
              event.preventDefault()
              move(event.key === 'ArrowDown' ? 1 : -1)
            } else if (event.key === 'Enter') {
              event.preventDefault()
              const entry = entries[current]
              if (onSubmit && (!entry || hasModifier(event))) onSubmit()
              else void run(entry)
            } else if (event.key === 'Tab' && onTab) {
              event.preventDefault()
              onTab(entries[current])
            } else if (event.key === 'Backspace' && onBack && query === '') {
              event.preventDefault()
              onBack()
            }
          }}
          placeholder={placeholder}
          aria-label={placeholder}
          role='combobox'
          aria-expanded
          aria-controls='palette-results'
          aria-activedescendant={
            entries[current] ? `palette-${current}` : undefined
          }
          className='placeholder:text-muted-foreground min-w-0 flex-1 bg-transparent text-sm outline-none'
        />
        {accessory}
      </label>
      {status}
      {children}
      <div
        ref={list}
        id='palette-results'
        role='listbox'
        aria-busy={busy !== null}
        className='max-h-[min(26rem,60vh)] overflow-y-auto p-1.5 empty:hidden'
      >
        {groups.length === 0 ? (
          empty ? (
            <div className='text-muted-foreground px-3 py-6 text-center text-sm text-pretty'>
              {empty}
            </div>
          ) : null
        ) : (
          groups.map((group, groupAt) => (
            // biome-ignore lint/a11y/useSemanticElements: a listbox group, not a form fieldset
            <div
              key={group.label}
              role='group'
              aria-label={group.label || undefined}
            >
              {group.label ? (
                <p className='text-muted-foreground px-2.5 pt-2 pb-1 text-[0.6875rem] font-semibold tracking-wide uppercase'>
                  {group.label}
                </p>
              ) : null}
              {group.entries.map((entry, entryAt) => {
                const at = (firstIndex[groupAt] ?? 0) + entryAt
                return (
                  // biome-ignore lint/a11y/useKeyWithClickEvents: the search input drives the keyboard
                  <div
                    key={entry.key}
                    id={`palette-${at}`}
                    data-index={at}
                    role='option'
                    tabIndex={-1}
                    aria-selected={at === current}
                    aria-disabled={
                      entry.disabled || (busy !== null && busy !== entry.key)
                    }
                    onMouseMove={() => {
                      if (at !== current) setActive(at)
                    }}
                    onClick={() => void run(entry)}
                    className={cn(
                      'flex min-h-9 cursor-pointer items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-sm',
                      at === current && 'bg-accent text-accent-foreground',
                      entry.disabled && 'cursor-default',
                      busy === entry.key && 'animate-pulse',
                      busy !== null && busy !== entry.key && 'opacity-50'
                    )}
                  >
                    {entry.content}
                  </div>
                )
              })}
              {group.divider ? (
                <div className='bg-border mx-2.5 my-1.5 h-px' />
              ) : null}
            </div>
          ))
        )}
      </div>
      <div className='border-border text-muted-foreground flex items-center gap-4 border-t px-4 py-2 text-xs'>
        <span className='flex items-center gap-1.5'>
          <Kbd>↑</Kbd>
          <Kbd>↓</Kbd>
          {m.palette_navigate()}
        </span>
        <span className='flex items-center gap-1.5'>
          <Kbd>↵</Kbd>
          {enterLabel ?? m.palette_open()}
        </span>
        {onBack ? (
          <span className='flex items-center gap-1.5'>
            <Kbd>⌫</Kbd>
            {m.palette_back()}
          </span>
        ) : null}
        <span className='flex items-center gap-1.5'>
          <Kbd>Esc</Kbd>
          {m.palette_close()}
        </span>
        {footer ? <span className='ml-auto'>{footer}</span> : null}
      </div>
    </>
  )
}
