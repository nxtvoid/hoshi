import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { useAppStore } from '@/state/app-store'
import type { Effort } from '@/bindings/Effort'
import { HugeiconsIcon } from '@hugeicons/react'
import { Kbd } from '@hoshi/ui/components/kbd'
import { useRef, useState } from 'react'
import { effectiveEffort } from '@/lib/model-runs'
import { useModelsStore } from '@/state/models-store'
import type { ModelInfo } from '@/bindings/ModelInfo'
import { Button } from '@hoshi/ui/components/button'
import { useSettingsStore } from '@/state/settings-store'
import { useModelPickerStore } from '@/state/model-picker-store'
import type { FavoriteModel } from '@/bindings/FavoriteModel'
import type { ProviderModels } from '@/bindings/ProviderModels'
import { metaOf } from '@/components/brand/provider-meta'
import { EFFORT_LABELS, EFFORT_SHORT_LABELS } from './effort-labels'
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from 'react'
import { type ModelChoice, useComposerStore } from '@/state/composer-store'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'
import {
  Popover,
  PopoverContent,
  PopoverTrigger
} from '@hoshi/ui/components/popover'
import {
  ArrowDown01Icon,
  FlashIcon,
  Search01Icon,
  StarIcon,
  Tick02Icon,
  WorkflowSquare03Icon
} from '@hugeicons/core-free-icons'

const EFFORT_COLUMNS: Record<number, string> = {
  1: 'grid-cols-1',
  2: 'grid-cols-2',
  3: 'grid-cols-3',
  4: 'grid-cols-4',
  5: 'grid-cols-5'
}

const SHORTCUT_COUNT = 9
const FAVORITES = 'favorites'
const NO_FAVORITES: FavoriteModel[] = []

type Row = { provider: string; model: ModelInfo }

const isFavorite = (favorites: readonly FavoriteModel[], row: Row) =>
  favorites.some(
    (favorite) =>
      favorite.provider === row.provider && favorite.model === row.model.slug
  )

function matches(model: ModelInfo, query: string) {
  const haystack = `${model.name} ${model.shortName}`.toLowerCase()
  return query.split(/\s+/).every((token) => haystack.includes(token))
}

export function ModelPicker() {
  const thread = useAppStore((state) =>
    state.threads.find((candidate) => candidate.id === state.selectedThreadId)
  )
  const draftModel = useComposerStore((state) => state.model)
  const draftEffort = useComposerStore((state) => state.effort)
  const draftFastMode = useComposerStore((state) => state.fastMode)
  const draftUltracode = useComposerStore((state) => state.ultracode)
  const setModel = useAppStore((state) => state.setModel)
  const catalog = useModelsStore((state) => state.catalog)
  const favorites = useSettingsStore(
    (state) => state.settings?.favoriteModels ?? NO_FAVORITES
  )
  const toggleFavorite = useSettingsStore((state) => state.toggleFavoriteModel)

  const choice: ModelChoice = thread
    ? {
        model: thread.model,
        effort: thread.effort,
        fastMode: thread.fastMode,
        ultracode: thread.ultracode
      }
    : {
        model: draftModel,
        effort: draftEffort,
        fastMode: draftFastMode,
        ultracode: draftUltracode
      }
  const providers: ProviderModels[] = catalog ? [catalog] : []
  const find = (slug: string | null | undefined): Row | undefined => {
    for (const provider of providers) {
      const model = provider.models.find((candidate) => candidate.slug === slug)
      if (model) return { provider: provider.provider, model }
    }
    return undefined
  }
  const selectedSlug = choice.model ?? catalog?.defaultModel ?? null
  const selected = find(selectedSlug)
  const meta = metaOf(selected?.provider ?? catalog?.provider)
  const effort = effectiveEffort(selected?.model, choice.effort)

  const open = useModelPickerStore((state) => state.open)
  const setOpen = useModelPickerStore((state) => state.setOpen)
  const [wasOpen, setWasOpen] = useState(open)
  const [tab, setTab] = useState<string>(FAVORITES)
  const [query, setQuery] = useState('')
  const [showLegacy, setShowLegacy] = useState(false)
  const searchRef = useRef<HTMLInputElement>(null)

  const update = (change: Partial<ModelChoice>) => {
    const next = { ...choice, ...change }
    const model = find(next.model ?? catalog?.defaultModel)?.model
    if (!model) return void setModel(next)
    void setModel({
      ...next,
      effort:
        next.effort && model.efforts.includes(next.effort) ? next.effort : null,
      fastMode: next.fastMode && model.fastMode,
      ultracode: next.ultracode && model.ultracode
    })
  }

  if (open !== wasOpen) {
    setWasOpen(open)
    if (open) {
      setQuery('')
      setTab(
        favorites.length > 0
          ? FAVORITES
          : (selected?.provider ?? providers[0]?.provider ?? FAVORITES)
      )
      setShowLegacy(selected?.model.legacy ?? false)
    }
  }

  const pick = (row: Row) => {
    update({ model: row.model.slug })
    setOpen(false)
  }

  const needle = query.trim().toLowerCase()
  const allRows: Row[] = providers.flatMap((provider) =>
    provider.models.map((model) => ({ provider: provider.provider, model }))
  )
  const browsed = providers.find((provider) => provider.provider === tab)
  let rows: Row[]
  let hiddenLegacy = 0
  if (needle) {
    const found = allRows.filter((row) => matches(row.model, needle))
    rows = [
      ...found.filter((row) => isFavorite(favorites, row)),
      ...found.filter((row) => !isFavorite(favorites, row))
    ]
  } else if (tab === FAVORITES) {
    rows = favorites.flatMap((favorite) => {
      const row = find(favorite.model)
      return row && row.provider === favorite.provider ? [row] : []
    })
  } else {
    const providerRows = allRows.filter(
      (row) => row.provider === browsed?.provider
    )
    const rest = providerRows.filter((row) => !isFavorite(favorites, row))
    const legacy = rest.filter((row) => row.model.legacy)
    hiddenLegacy = legacy.length
    rows = [
      ...providerRows.filter((row) => isFavorite(favorites, row)),
      ...rest.filter((row) => !row.model.legacy),
      ...(showLegacy ? legacy : [])
    ]
  }
  const shortcutRows = rows
    .filter((row) => !row.model.updateRequired)
    .slice(0, SHORTCUT_COUNT)

  const onKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (!(event.ctrlKey || event.metaKey) || event.altKey || event.shiftKey)
      return
    const row = shortcutRows[Number(event.key) - 1]
    if (!row) return
    event.preventDefault()
    pick(row)
  }

  const onSearchKeyDown = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (event.key !== 'Enter') return
    const first = shortcutRows[0]
    if (first) pick(first)
  }

  const selectedModel = selected?.model

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        render={<Button variant='ghost' size='xs' className='-mr-1.5' />}
      >
        <meta.icon className={cn('size-3', meta.tint)} />
        <span className='text-foreground font-medium'>
          {selectedModel?.shortName ?? choice.model ?? meta.label()}
        </span>
        {effort ? <span>{EFFORT_LABELS[effort]()}</span> : null}
        {choice.fastMode ? (
          <HugeiconsIcon
            icon={FlashIcon}
            strokeWidth={2}
            aria-label={m.speed_fast()}
            className={meta.tint}
          />
        ) : null}
        {choice.ultracode ? (
          <HugeiconsIcon
            icon={WorkflowSquare03Icon}
            strokeWidth={2}
            aria-label={m.ultracode()}
            className={meta.tint}
          />
        ) : null}
        <HugeiconsIcon icon={ArrowDown01Icon} strokeWidth={2} />
      </PopoverTrigger>
      <PopoverContent
        side='top'
        align='end'
        initialFocus={searchRef}
        onKeyDown={onKeyDown}
        className='w-96 gap-0 overflow-hidden rounded-xl p-0'
      >
        <div className='flex'>
          <div className='border-border bg-muted/40 flex w-11 shrink-0 flex-col items-center gap-1 border-r py-1.5'>
            <RailButton
              label={m.model_favorites()}
              active={!needle && tab === FAVORITES}
              activeClassName='text-favorite'
              onSelect={() => {
                setTab(FAVORITES)
                setQuery('')
              }}
            >
              <HugeiconsIcon
                icon={StarIcon}
                strokeWidth={2}
                fill='currentColor'
                className='size-4'
              />
            </RailButton>
            <span className='bg-border my-0.5 h-px w-4' />
            {providers.map((provider) => {
              const providerMeta = metaOf(provider.provider)
              return (
                <RailButton
                  key={provider.provider}
                  label={providerMeta.label()}
                  active={!needle && tab === provider.provider}
                  activeClassName={providerMeta.tint}
                  onSelect={() => {
                    setTab(provider.provider)
                    setQuery('')
                  }}
                >
                  <providerMeta.icon className='size-4' />
                </RailButton>
              )
            })}
          </div>
          <div className='min-w-0 flex-1'>
            <label className='border-border text-muted-foreground flex h-9 items-center gap-2 border-b px-3'>
              <HugeiconsIcon
                icon={Search01Icon}
                strokeWidth={2}
                className='size-3.5 shrink-0'
              />
              <input
                ref={searchRef}
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                onKeyDown={onSearchKeyDown}
                placeholder={m.search_models()}
                className='text-foreground placeholder:text-muted-foreground min-w-0 flex-1 bg-transparent text-sm outline-none'
              />
            </label>
            <div className='max-h-80 overflow-y-auto p-1'>
              <p className='text-muted-foreground px-2 pt-1.5 pb-1 text-xs font-medium'>
                {needle
                  ? m.search_results()
                  : tab === FAVORITES
                    ? m.model_favorites()
                    : metaOf(browsed?.provider).label()}
              </p>
              {rows.length === 0 ? (
                <p className='text-muted-foreground px-2 py-6 text-center text-xs'>
                  {needle
                    ? m.no_matching_models()
                    : tab === FAVORITES
                      ? m.no_favorite_models()
                      : m.no_models()}
                </p>
              ) : null}
              {rows.map((row) => {
                const rowMeta = metaOf(row.provider)
                const shortcut = shortcutRows.indexOf(row)
                return (
                  <ModelRow
                    key={`${row.provider}:${row.model.slug}`}
                    model={row.model}
                    icon={<rowMeta.icon />}
                    tile={rowMeta.tile}
                    selected={row.model.slug === selectedSlug}
                    favorite={isFavorite(favorites, row)}
                    shortcut={
                      shortcut >= 0
                        ? m.model_shortcut({ key: String(shortcut + 1) })
                        : undefined
                    }
                    onSelect={() => pick(row)}
                    onToggleFavorite={() =>
                      void toggleFavorite({
                        provider: row.provider,
                        model: row.model.slug
                      })
                    }
                  />
                )
              })}
              {hiddenLegacy > 0 ? (
                <button
                  type='button'
                  onClick={() => setShowLegacy(!showLegacy)}
                  className='text-muted-foreground hover:text-foreground focus-visible:bg-accent w-full rounded-lg px-2 py-1.5 text-left text-xs outline-none'
                >
                  {showLegacy ? m.model_show_less() : m.model_show_more()}
                </button>
              ) : null}
            </div>
          </div>
        </div>
        {selectedModel ? (
          <ModelOptions
            model={selectedModel}
            choice={choice}
            effort={effort}
            tint={meta.tint}
            onChange={update}
          />
        ) : null}
      </PopoverContent>
    </Popover>
  )
}

type RailButtonProps = {
  label: string
  active: boolean
  /** The color of the icon while its tab is open. */
  activeClassName: string
  onSelect: () => void
  children: ReactNode
}

function RailButton({
  label,
  active,
  activeClassName,
  onSelect,
  children
}: RailButtonProps) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type='button'
            aria-label={label}
            aria-pressed={active}
            onClick={onSelect}
            className={cn(
              'focus-visible:ring-ring grid size-8 place-items-center rounded-lg transition-colors outline-none focus-visible:ring-2',
              active
                ? cn('bg-popover ring-border shadow-xs ring-1', activeClassName)
                : 'text-muted-foreground hover:text-foreground'
            )}
          />
        }
      >
        {children}
      </TooltipTrigger>
      <TooltipContent side='left'>{label}</TooltipContent>
    </Tooltip>
  )
}

type ModelRowProps = {
  model: ModelInfo
  icon: ReactNode
  tile: string
  selected: boolean
  favorite: boolean
  shortcut?: string
  onSelect: () => void
  onToggleFavorite: () => void
}

function ModelRow({
  model,
  icon,
  tile,
  selected,
  favorite,
  shortcut,
  onSelect,
  onToggleFavorite
}: ModelRowProps) {
  return (
    <div
      className={cn(
        'group hover:bg-accent/60 focus-within:bg-accent/60 flex items-center gap-1 rounded-lg pr-2',
        selected && 'bg-accent'
      )}
    >
      <button
        type='button'
        aria-pressed={selected}
        disabled={model.updateRequired}
        onClick={onSelect}
        className='flex min-w-0 flex-1 items-center gap-2.5 py-1.5 pl-2 text-left outline-none disabled:opacity-50'
      >
        <span
          className={cn(
            'grid size-7 shrink-0 place-items-center rounded-md [&_svg]:size-3.5',
            tile
          )}
        >
          {icon}
        </span>
        <span className='min-w-0 truncate text-sm font-medium'>
          {model.name}
        </span>
        {model.isNew ? (
          <span className='bg-primary text-primary-foreground shrink-0 rounded px-1 text-[10px] font-medium'>
            {m.model_new()}
          </span>
        ) : null}
        {model.updateRequired ? (
          <span className='text-muted-foreground shrink-0 text-xs'>
            {m.model_update_required()}
          </span>
        ) : null}
        {selected ? (
          <HugeiconsIcon
            icon={Tick02Icon}
            strokeWidth={2}
            className='ml-auto size-4 shrink-0'
          />
        ) : null}
      </button>
      <button
        type='button'
        aria-label={favorite ? m.remove_favorite() : m.add_favorite()}
        aria-pressed={favorite}
        onClick={onToggleFavorite}
        className={cn(
          'focus-visible:ring-ring grid size-6 shrink-0 place-items-center rounded-md outline-none focus-visible:ring-2',
          favorite
            ? 'text-favorite'
            : 'text-muted-foreground hover:text-foreground opacity-0 group-focus-within:opacity-100 group-hover:opacity-100'
        )}
      >
        <HugeiconsIcon
          icon={StarIcon}
          strokeWidth={2}
          fill={favorite ? 'currentColor' : 'none'}
          className='size-3.5'
        />
      </button>
      {shortcut ? (
        <Kbd className='border-border border bg-transparent'>{shortcut}</Kbd>
      ) : null}
    </div>
  )
}

type ModelOptionsProps = {
  model: ModelInfo
  choice: ModelChoice
  effort: Effort | null
  /** The provider's color, for modes that are on. */
  tint: string
  onChange: (change: Partial<ModelChoice>) => void
}

function ModelOptions({
  model,
  choice,
  effort,
  tint,
  onChange
}: ModelOptionsProps) {
  if (model.efforts.length === 0 && !model.fastMode && !model.ultracode)
    return null
  return (
    <div className='border-border bg-muted/40 flex flex-col gap-2.5 border-t px-3 pt-2.5 pb-3'>
      {model.efforts.length > 0 ? (
        <EffortPicker
          efforts={model.efforts}
          defaultEffort={model.defaultEffort}
          effort={effort}
          onChange={(next) => onChange({ effort: next })}
        />
      ) : null}
      {model.fastMode || model.ultracode ? (
        <ModePicker
          choice={choice}
          fastMode={model.fastMode}
          ultracode={model.ultracode}
          tint={tint}
          onChange={onChange}
        />
      ) : null}
    </div>
  )
}

type EffortPickerProps = {
  efforts: Effort[]
  defaultEffort: Effort | null
  effort: Effort | null
  onChange: (effort: Effort) => void
}

function EffortPicker({
  efforts,
  defaultEffort,
  effort,
  onChange
}: EffortPickerProps) {
  return (
    <div>
      <div className='text-muted-foreground mb-2 flex items-center justify-between text-xs'>
        <span>{m.effort_label()}</span>
        {effort ? (
          <span className='flex items-center gap-1.5'>
            {effort === defaultEffort ? (
              <span className='border-border rounded border px-1 text-[10px]'>
                {m.effort_default_tag()}
              </span>
            ) : null}
            <span className='text-foreground font-medium'>
              {EFFORT_LABELS[effort]()}
            </span>
          </span>
        ) : null}
      </div>
      <fieldset
        aria-label={m.effort_label()}
        className={cn(
          'bg-muted grid gap-0.5 rounded-lg p-0.5',
          EFFORT_COLUMNS[efforts.length] ?? 'grid-cols-5'
        )}
      >
        {efforts.map((option, index) => (
          <button
            key={option}
            type='button'
            aria-pressed={option === effort}
            onClick={() => onChange(option)}
            className={cn(
              'focus-visible:ring-ring flex flex-col items-center gap-1 rounded-md pt-1.5 pb-1 text-[10px] outline-none focus-visible:ring-2',
              option === effort
                ? 'bg-popover text-foreground shadow-xs'
                : 'text-muted-foreground hover:text-foreground'
            )}
          >
            <EffortMeter level={index + 1} levels={efforts.length} />
            {EFFORT_SHORT_LABELS[option]()}
          </button>
        ))}
      </fieldset>
    </div>
  )
}

type ModePickerProps = {
  choice: ModelChoice
  fastMode: boolean
  ultracode: boolean
  /** The provider's color, for modes that are on. */
  tint: string
  onChange: (change: Partial<ModelChoice>) => void
}

function ModePicker({
  choice,
  fastMode,
  ultracode,
  tint,
  onChange
}: ModePickerProps) {
  const speeds = [
    { fastMode: false, label: m.speed_standard(), icon: null },
    { fastMode: true, label: m.speed_fast(), icon: FlashIcon }
  ]
  return (
    <div className='flex items-center gap-2'>
      {fastMode ? (
        <fieldset
          aria-label={m.speed_label()}
          className='bg-muted grid flex-1 grid-cols-2 gap-0.5 rounded-lg p-0.5'
        >
          {speeds.map((speed) => {
            const selected = choice.fastMode === speed.fastMode
            return (
              <button
                key={speed.label}
                type='button'
                aria-pressed={selected}
                onClick={() => onChange({ fastMode: speed.fastMode })}
                className={cn(
                  'focus-visible:ring-ring flex h-6 items-center justify-center gap-1 rounded-md text-xs outline-none focus-visible:ring-2',
                  selected
                    ? 'bg-popover text-foreground font-medium shadow-xs'
                    : 'text-muted-foreground hover:text-foreground'
                )}
              >
                {speed.icon ? (
                  <HugeiconsIcon
                    icon={speed.icon}
                    strokeWidth={2}
                    className={cn('size-3.5', selected && tint)}
                  />
                ) : null}
                {speed.label}
              </button>
            )
          })}
        </fieldset>
      ) : null}
      {ultracode ? (
        <button
          type='button'
          aria-pressed={choice.ultracode}
          onClick={() => onChange({ ultracode: !choice.ultracode })}
          className={cn(
            'focus-visible:ring-ring ring-border flex h-7 items-center gap-1 rounded-lg px-2.5 text-xs ring-1 outline-none focus-visible:ring-2',
            !fastMode && 'flex-1 justify-center',
            choice.ultracode
              ? 'bg-popover text-foreground font-medium shadow-xs'
              : 'text-muted-foreground hover:text-foreground'
          )}
        >
          <HugeiconsIcon
            icon={WorkflowSquare03Icon}
            strokeWidth={2}
            className={cn('size-3.5', choice.ultracode && tint)}
          />
          {m.ultracode()}
        </button>
      ) : null}
    </div>
  )
}

const BAR_HEIGHTS = ['h-1', 'h-1.5', 'h-2', 'h-2.5', 'h-3'] as const

/** `level` bars lit out of the model's `levels` effort options. */
function EffortMeter({ level, levels }: { level: number; levels: number }) {
  const lit = Math.round((level / levels) * BAR_HEIGHTS.length)
  return (
    <span className='flex h-3 items-end gap-px'>
      {BAR_HEIGHTS.map((height, index) => (
        <span
          key={height}
          className={cn(
            'w-0.5 rounded-full bg-current',
            height,
            index >= lit && 'opacity-25'
          )}
        />
      ))}
    </span>
  )
}
