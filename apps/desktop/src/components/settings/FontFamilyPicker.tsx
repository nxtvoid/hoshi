import { m } from '@/paraglide/messages'
import { useCallback, useMemo, useRef, useState } from 'react'
import { useSystemFontsStore } from '@/state/system-fonts-store'
import {
  type FontOption,
  fontPickerValues,
  installedFamilies
} from '@/lib/fonts'
import { cssFontFamily, type FontSlot, isFontPreset } from '@/state/appearance'
import {
  Combobox,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxItem,
  ComboboxList,
  ComboboxSearch,
  ComboboxStatus,
  ComboboxTrigger
} from '@hoshi/ui/components/combobox'

const ROW_HEIGHT = 28
const VISIBLE_ROWS = 10
const OVERSCAN = 6
const LIST_PADDING = 4

const PRESET_STACKS: Record<string, string> = {
  interface: 'var(--font-sans)',
  geist: 'var(--stack-geist)',
  inter: 'var(--stack-inter)',
  jetbrains: 'var(--stack-jetbrains)',
  cascadia: 'var(--stack-cascadia)',
  'system-mono': 'var(--stack-system-mono)'
}

export function isMonospaceSlot(slot: FontSlot): boolean {
  return slot === 'code' || slot === 'terminal'
}

function itemStack(slot: FontSlot, value: string): string {
  const monospace = isMonospaceSlot(slot)
  if (!isFontPreset(slot, value)) {
    return `${cssFontFamily(value)}, ${monospace ? 'monospace' : 'sans-serif'}`
  }
  if (value === 'system') {
    return monospace ? 'var(--stack-system-mono)' : 'var(--stack-system)'
  }
  return PRESET_STACKS[value] ?? 'inherit'
}

/** Picks a preset or any installed family; code and terminal only list fixed-width ones. */
export function FontFamilyPicker({
  id,
  slot,
  value,
  presets,
  labelOf,
  onChange
}: {
  id: string
  slot: FontSlot
  value: string
  /** The default first, then the fonts hoshi ships. */
  presets: readonly FontOption[]
  labelOf: (value: string) => string
  onChange: (value: string) => void
}) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [scrollTop, setScrollTop] = useState(0)
  const listRef = useRef<HTMLDivElement | null>(null)
  const openingScroll = useRef(0)
  const fonts = useSystemFontsStore((state) => state.fonts)
  const families = useMemo(
    () => (fonts ? installedFamilies(fonts, isMonospaceSlot(slot)) : []),
    [fonts, slot]
  )
  const items = fontPickerValues(presets, families, query)
  const defaultValue = presets[0]?.value

  const attachList = useCallback((element: HTMLDivElement | null) => {
    listRef.current = element
    if (element) element.scrollTop = openingScroll.current
  }, [])

  const scrollTo = (top: number) => {
    openingScroll.current = top
    setScrollTop(top)
    if (listRef.current) listRef.current.scrollTop = top
  }

  const reveal = (index: number) => {
    const list = listRef.current
    if (!list || index < 0) return
    const top = index * ROW_HEIGHT
    const bottom = top + ROW_HEIGHT + 2 * LIST_PADDING
    if (top < list.scrollTop) scrollTo(top)
    else if (bottom > list.scrollTop + list.clientHeight) {
      scrollTo(bottom - list.clientHeight)
    }
  }

  const handleOpenChange = (next: boolean) => {
    setOpen(next)
    if (!next) return
    setQuery('')
    const index = fontPickerValues(presets, families, '').indexOf(value)
    openingScroll.current =
      Math.max(0, index - Math.floor(VISIBLE_ROWS / 2)) * ROW_HEIGHT
    setScrollTop(openingScroll.current)
  }

  const viewport = Math.min(items.length, VISIBLE_ROWS) * ROW_HEIGHT
  const first = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN)
  const last = Math.min(
    items.length,
    Math.ceil((scrollTop + viewport) / ROW_HEIGHT) + OVERSCAN
  )

  return (
    <Combobox
      items={items}
      filteredItems={items}
      virtualized
      autoHighlight
      open={open}
      onOpenChange={handleOpenChange}
      inputValue={query}
      onInputValueChange={(next) => {
        setQuery(next)
        scrollTo(0)
      }}
      value={value}
      onValueChange={(next: string | null) => {
        if (next !== null && next !== value) onChange(next)
      }}
      itemToStringLabel={labelOf}
      onItemHighlighted={(item, { reason, index }) => {
        if (item !== undefined && reason !== 'pointer') reveal(index)
      }}
    >
      <ComboboxTrigger id={id} className='w-44'>
        {labelOf(value)}
      </ComboboxTrigger>
      <ComboboxContent align='end'>
        <ComboboxSearch
          placeholder={m.search_fonts()}
          aria-label={m.search_fonts()}
        />
        {fonts === null ? null : (
          <ComboboxEmpty>{m.no_fonts_found()}</ComboboxEmpty>
        )}
        {items.length > 0 ? (
          <ComboboxList
            ref={attachList}
            onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
            className='overflow-y-auto overscroll-contain p-1'
            style={{ height: viewport + 2 * LIST_PADDING }}
          >
            <div
              role='presentation'
              className='relative'
              style={{ height: items.length * ROW_HEIGHT }}
            >
              {items.slice(first, last).map((item, offset) => {
                const index = first + offset
                return (
                  <ComboboxItem
                    key={item}
                    value={item}
                    index={index}
                    aria-setsize={items.length}
                    aria-posinset={index + 1}
                    className='absolute inset-x-0'
                    style={{ top: index * ROW_HEIGHT, height: ROW_HEIGHT }}
                  >
                    <span
                      className='min-w-0 truncate'
                      style={{ fontFamily: itemStack(slot, item) }}
                    >
                      {labelOf(item)}
                    </span>
                    {item === defaultValue ? (
                      <span className='text-muted-foreground shrink-0 text-xs'>
                        {m.font_default()}
                      </span>
                    ) : null}
                  </ComboboxItem>
                )
              })}
            </div>
          </ComboboxList>
        ) : null}
        {fonts === null ? (
          <ComboboxStatus>{m.loading_fonts()}</ComboboxStatus>
        ) : null}
      </ComboboxContent>
    </Combobox>
  )
}
