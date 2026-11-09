import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { HugeiconsIcon } from '@hugeicons/react'
import { type ReactNode, useState } from 'react'
import { Button } from '@hoshi/ui/components/button'
import { useThemeEditorStore } from '@/state/theme-editor-store'
import { useResolvedAppearance, useThemeStore } from '@/state/theme-store'
import { THEME_OPTIONS } from './theme-options'
import type { ThemePreference } from '@/state/appearance'
import { ThemeImportDialog } from './ThemeImportDialog'
import { ThemeOrb, ThemeWireframe } from './ThemePreviews'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle
} from '@hoshi/ui/components/alert-dialog'
import {
  Add01Icon,
  Copy01Icon,
  Delete02Icon,
  Moon02Icon,
  PaintBrush01Icon,
  PencilEdit02Icon,
  Sun03Icon,
  Upload04Icon
} from '@hugeicons/core-free-icons'
import {
  BUILT_IN_THEMES,
  chooseCard,
  chooseHalf,
  chooseTheme,
  findTheme,
  halfOwners,
  removeThemes,
  selectionWithout,
  serializeThemeFile,
  STANDARD_THEME_ID,
  THEME_APPEARANCES,
  type ThemeAppearance,
  themeColorsFor,
  type ThemeDefinition,
  themeModes
} from '@/state/themes'

function useSelection() {
  const theme = useThemeStore((state) => state.theme)
  const halves = useThemeStore((state) => state.halves)
  return { theme, halves }
}

function useOwners() {
  const selection = useSelection()
  const customThemes = useThemeStore((state) => state.customThemes)
  return halfOwners(selection, customThemes)
}

function ownerColors(
  owner: string | null,
  appearance: ThemeAppearance,
  customThemes: readonly ThemeDefinition[]
) {
  const theme =
    findTheme(owner ?? STANDARD_THEME_ID, customThemes) ?? BUILT_IN_THEMES[0]!
  return themeColorsFor(theme, appearance) ?? theme.colors
}

export function ColorSchemeTiles() {
  const preference = useThemeStore((state) => state.preference)
  const setPreference = useThemeStore((state) => state.setPreference)
  const customThemes = useThemeStore((state) => state.customThemes)
  const owners = useOwners()
  const colors = (appearance: ThemeAppearance) =>
    ownerColors(owners[appearance], appearance, customThemes)

  return (
    <fieldset className='grid grid-cols-3 gap-3'>
      <legend className='sr-only'>{m.color_scheme()}</legend>
      {THEME_OPTIONS.map(({ value, label, icon }) => {
        const selected = preference === value
        return (
          <button
            key={value}
            type='button'
            aria-pressed={selected}
            onClick={() => setPreference(value)}
            className={cn(
              'focus-visible:ring-ring flex flex-col gap-2 rounded-xl border p-2 text-sm font-medium transition-colors outline-none focus-visible:ring-2',
              selected
                ? 'border-primary ring-primary text-foreground ring-1'
                : 'text-muted-foreground hover:bg-muted/50 hover:text-foreground'
            )}
          >
            <SchemePreview mode={value} colors={colors} />
            <span className='flex items-center gap-1.5 px-1'>
              <HugeiconsIcon icon={icon} strokeWidth={2} className='size-4' />
              {label()}
            </span>
          </button>
        )
      })}
    </fieldset>
  )
}

function SchemePreview({
  mode,
  colors
}: {
  mode: ThemePreference
  colors: (appearance: ThemeAppearance) => ThemeDefinition['colors']
}) {
  return (
    <span className='border-border relative block h-16 overflow-hidden rounded-md border'>
      {mode === 'system' ? (
        <>
          <ThemeWireframe
            colors={colors('light')}
            appearance='light'
            clip='left'
          />
          <ThemeWireframe
            colors={colors('dark')}
            appearance='dark'
            clip='right'
          />
        </>
      ) : (
        <ThemeWireframe colors={colors(mode)} appearance={mode} />
      )}
    </span>
  )
}

type Removal = {
  theme: ThemeDefinition
  collection: readonly ThemeDefinition[]
}

/** Custom themes grouped by the extension that installed them; a theme of its own stands alone. */
function collectionsOf(themes: readonly ThemeDefinition[]) {
  const groups = new Map<string, ThemeDefinition[]>()
  for (const theme of themes) {
    const key = theme.collection
      ? `collection:${theme.collection.id}`
      : `theme:${theme.id}`
    const group = groups.get(key)
    if (group) group.push(theme)
    else groups.set(key, [theme])
  }
  return [...groups.entries()]
}

export function ThemeGrid() {
  const selection = useSelection()
  const customThemes = useThemeStore((state) => state.customThemes)
  const setSelection = useThemeStore((state) => state.setSelection)
  const setCustomThemes = useThemeStore((state) => state.setCustomThemes)
  const openThemeEditor = useThemeEditorStore((state) => state.openThemeEditor)
  const initialAppearance = useResolvedAppearance()
  const [removal, setRemoval] = useState<Removal | null>(null)
  const [removalOpen, setRemovalOpen] = useState(false)
  const [toRemove, setToRemove] = useState<readonly string[]>([])
  const [notice, setNotice] = useState<string | null>(null)
  const owners = halfOwners(selection, customThemes)
  const pickedModes = (cardId: string | null) =>
    THEME_APPEARANCES.filter((appearance) => owners[appearance] === cardId)

  const duplicate = (theme: ThemeDefinition) =>
    openThemeEditor({
      editingThemeId: null,
      seedThemeId: theme.id === STANDARD_THEME_ID ? null : theme.id,
      seedName: m.theme_copy_name({ name: theme.label }),
      initialAppearance
    })

  const edit = (theme: ThemeDefinition) =>
    openThemeEditor({
      editingThemeId: theme.id,
      seedThemeId: null,
      seedName: null,
      initialAppearance
    })

  const exportTheme = (theme: ThemeDefinition) => {
    invoke<boolean>('export_theme', {
      fileName: `${theme.id}.json`,
      contents: serializeThemeFile(theme)
    }).catch(() => setNotice(m.export_theme_failed()))
  }

  const askRemoval = (
    theme: ThemeDefinition,
    collection: readonly ThemeDefinition[]
  ) => {
    setRemoval({ theme, collection })
    setToRemove(collection.length > 1 ? [] : [theme.id])
    setRemovalOpen(true)
  }

  const confirmRemoval = () => {
    if (toRemove.length === 0) return
    const { customThemes: current, theme, halves } = useThemeStore.getState()
    setCustomThemes(
      removeThemes(current, toRemove),
      selectionWithout({ theme, halves }, toRemove)
    )
    setRemovalOpen(false)
  }

  const removingCollection = (removal?.collection.length ?? 0) > 1

  return (
    <>
      <div className='grid grid-cols-[repeat(auto-fill,minmax(min(100%,13rem),1fr))] gap-2'>
        {BUILT_IN_THEMES.map((theme) => {
          const cardId = theme.id === STANDARD_THEME_ID ? null : theme.id
          return (
            <ThemeCard
              key={theme.id}
              theme={theme}
              pickedModes={pickedModes(cardId)}
              onUse={() => setSelection(chooseTheme(theme.id))}
              onUseMode={(appearance) =>
                setSelection(chooseHalf(selection, appearance, cardId))
              }
              onDuplicate={() => duplicate(theme)}
            />
          )
        })}
        {collectionsOf(customThemes).map(([key, themes]) =>
          themes.length > 1 ? (
            <CollectionCard
              key={key}
              themes={themes}
              pickedModes={pickedModes}
              onUseMode={(theme, appearance) =>
                setSelection(
                  chooseHalf(useSelectionNow(), appearance, theme.id)
                )
              }
              onDuplicate={duplicate}
              onEdit={edit}
              onExport={exportTheme}
              onRemove={(theme) => askRemoval(theme, themes)}
            />
          ) : (
            <ThemeCard
              key={key}
              theme={themes[0]!}
              pickedModes={pickedModes(themes[0]!.id)}
              onUse={() => setSelection(chooseCard(selection, themes[0]!))}
              onUseMode={(appearance) =>
                setSelection(chooseHalf(selection, appearance, themes[0]!.id))
              }
              onDuplicate={() => duplicate(themes[0]!)}
              onEdit={() => edit(themes[0]!)}
              onExport={() => exportTheme(themes[0]!)}
              onRemove={() => askRemoval(themes[0]!, themes)}
            />
          )
        )}
      </div>
      <AlertDialog open={removalOpen} onOpenChange={setRemovalOpen}>
        <AlertDialogContent className='data-[size=default]:sm:max-w-md'>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {removingCollection
                ? m.remove_themes_from_confirm({
                    name:
                      removal?.theme.collection?.label ??
                      removal?.theme.label ??
                      ''
                  })
                : m.remove_theme_confirm({ name: removal?.theme.label ?? '' })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {removingCollection
                ? m.remove_themes_description()
                : m.remove_theme_description()}
            </AlertDialogDescription>
          </AlertDialogHeader>
          {removingCollection && removal ? (
            <div className='grid max-h-72 grid-cols-1 gap-2 overflow-y-auto sm:grid-cols-2'>
              {removal.collection.map((theme) => {
                const checked = toRemove.includes(theme.id)
                return (
                  <label
                    key={theme.id}
                    className={cn(
                      'border-border/70 bg-muted/20 hover:bg-muted/40 relative flex min-h-28 cursor-pointer flex-col items-center justify-center gap-2 rounded-lg border p-3',
                      checked && 'border-ring bg-accent/40'
                    )}
                  >
                    <input
                      type='checkbox'
                      checked={checked}
                      className='accent-primary absolute top-2 right-2 size-4'
                      onChange={(event) => {
                        const remove = event.currentTarget.checked
                        setToRemove((current) =>
                          remove
                            ? [...current, theme.id]
                            : current.filter((id) => id !== theme.id)
                        )
                      }}
                    />
                    <span className='flex items-center justify-center gap-1'>
                      {themeModes(theme).map((appearance) => (
                        <ThemeOrb
                          key={appearance}
                          colors={themeColorsFor(theme, appearance)!}
                          appearance={appearance}
                          className='size-10'
                        />
                      ))}
                    </span>
                    <span className='max-w-full truncate text-sm font-medium'>
                      {theme.label}
                    </span>
                  </label>
                )
              })}
            </div>
          ) : null}
          <AlertDialogFooter>
            <AlertDialogCancel>{m.cancel()}</AlertDialogCancel>
            <Button
              variant='destructive'
              disabled={toRemove.length === 0}
              onClick={confirmRemoval}
            >
              {!removingCollection
                ? m.remove_theme()
                : toRemove.length > 0
                  ? m.remove_selected_count({ count: toRemove.length })
                  : m.remove_selected()}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog
        open={notice !== null}
        onOpenChange={(open) => {
          if (!open) setNotice(null)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{notice}</AlertDialogTitle>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{m.ok()}</AlertDialogCancel>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

const useSelectionNow = () => {
  const { theme, halves } = useThemeStore.getState()
  return { theme, halves }
}

function ThemeCard({
  theme,
  pickedModes,
  onUse,
  onUseMode,
  onDuplicate,
  onEdit,
  onExport,
  onRemove,
  orbs,
  name = theme.label,
  removeLabel = m.remove_theme(),
  tooltip
}: {
  theme: ThemeDefinition
  pickedModes: readonly ThemeAppearance[]
  onUse: () => void
  onUseMode: (appearance: ThemeAppearance) => void
  onDuplicate: () => void
  onEdit?: () => void
  onExport?: () => void
  onRemove?: () => void
  orbs?: ReactNode
  name?: string
  removeLabel?: string
  tooltip?: string
}) {
  const modes = themeModes(theme)
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          // biome-ignore lint/a11y/useKeyWithClickEvents: the name inside is the keyboard target
          // biome-ignore lint/a11y/noStaticElementInteractions: the name inside is the keyboard target
          <div
            onClick={onUse}
            className='border-border/70 bg-card/60 hover:bg-muted/40 cursor-pointer overflow-hidden rounded-xl border transition-colors'
          />
        }
      >
        {orbs ?? (
          <ThemeOrbs
            theme={theme}
            modes={modes}
            pickedModes={pickedModes}
            onUseMode={onUseMode}
          />
        )}
        <div className='flex items-center gap-2 px-3 pt-2 pb-2.5'>
          <button
            type='button'
            aria-pressed={pickedModes.length > 0}
            aria-label={m.use_theme_named({ name })}
            onClick={(event) => {
              event.stopPropagation()
              onUse()
            }}
            className='focus-visible:ring-ring min-w-0 flex-1 cursor-pointer truncate rounded-sm text-left text-sm font-medium outline-none focus-visible:ring-2'
          >
            {name}
          </button>
          <div className='flex shrink-0 items-center gap-0.5'>
            <CardAction
              label={m.duplicate_theme()}
              icon={Copy01Icon}
              onClick={onDuplicate}
            />
            {onEdit ? (
              <CardAction
                label={m.edit_theme()}
                icon={PencilEdit02Icon}
                onClick={onEdit}
              />
            ) : null}
            {onExport ? (
              <CardAction
                label={m.export_theme()}
                icon={Upload04Icon}
                onClick={onExport}
              />
            ) : null}
            {onRemove ? (
              <CardAction
                label={removeLabel}
                icon={Delete02Icon}
                onClick={onRemove}
                destructive
              />
            ) : null}
          </div>
        </div>
      </TooltipTrigger>
      <TooltipContent>
        {tooltip ??
          (modes.length > 1
            ? m.use_theme_both()
            : modes[0] === 'light'
              ? m.use_theme_light_only()
              : m.use_theme_dark_only())}
      </TooltipContent>
    </Tooltip>
  )
}

/** The part of each variant's name that tells it apart from the others in its collection. */
function variantLabels(themes: readonly ThemeDefinition[]): string[] {
  const words = themes.map((theme) => theme.label.trim().split(/\s+/))
  const first = words[0] ?? []
  const shared = first.findIndex((word, index) =>
    words.some(
      (labelWords) =>
        labelWords[index]?.toLocaleLowerCase() !== word.toLocaleLowerCase()
    )
  )
  const prefix = shared === -1 ? first.length - 1 : shared
  return themes.map(
    (theme, index) =>
      words[index]?.slice(Math.max(0, prefix)).join(' ').trim() || theme.label
  )
}

function CollectionCard({
  themes,
  pickedModes,
  onUseMode,
  onDuplicate,
  onEdit,
  onExport,
  onRemove
}: {
  themes: readonly ThemeDefinition[]
  pickedModes: (cardId: string | null) => ThemeAppearance[]
  onUseMode: (theme: ThemeDefinition, appearance: ThemeAppearance) => void
  onDuplicate: (theme: ThemeDefinition) => void
  onEdit: (theme: ThemeDefinition) => void
  onExport: (theme: ThemeDefinition) => void
  onRemove: (theme: ThemeDefinition) => void
}) {
  const [variant, setVariant] = useState(() => {
    return Math.max(
      0,
      themes.findIndex((theme) => pickedModes(theme.id).length > 0)
    )
  })
  const [openMode, setOpenMode] = useState<ThemeAppearance | null>(null)
  const index = Math.min(variant, themes.length - 1)
  const theme = themes[index]!
  const labels = variantLabels(themes)
  const collectionLabel = theme.collection?.label ?? theme.label

  const useDefaults = () => {
    for (const appearance of THEME_APPEARANCES) {
      const first = themes.find((candidate) =>
        themeModes(candidate).includes(appearance)
      )
      if (first) onUseMode(first, appearance)
    }
    setVariant(0)
  }

  const groups = THEME_APPEARANCES.map((appearance) => {
    const options = themes.flatMap((candidate, themeIndex) =>
      themeModes(candidate).includes(appearance)
        ? [{ theme: candidate, themeIndex, label: labels[themeIndex]! }]
        : []
    )
    const selected =
      options.find((option) =>
        pickedModes(option.theme.id).includes(appearance)
      ) ?? options[0]
    return { appearance, options, selected }
  })

  const orbs = (
    <fieldset
      className='relative h-24'
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node)) {
          setOpenMode(null)
        }
      }}
      onKeyDown={(event) => {
        if (event.key === 'Escape') setOpenMode(null)
      }}
      onMouseLeave={() => setOpenMode(null)}
    >
      <legend className='sr-only'>{m.theme_variants()}</legend>
      {groups.map(({ appearance, options, selected }) => {
        if (!selected) return null
        const offset = appearance === 'light' ? -52 : 52
        const open = openMode === appearance
        const picked = pickedModes(selected.theme.id).includes(appearance)
        return (
          <div key={appearance} className='contents'>
            <div
              className='absolute top-2 left-1/2 z-20'
              style={{ transform: `translateX(calc(-50% + ${offset}px))` }}
            >
              <OrbButton
                onHover={() => setOpenMode(appearance)}
                theme={selected.theme}
                appearance={appearance}
                picked={picked}
                label={
                  appearance === 'light'
                    ? m.use_variant_light({ name: selected.label })
                    : m.use_variant_dark({ name: selected.label })
                }
                tooltip={
                  appearance === 'light'
                    ? m.variant_light_named({ name: selected.label })
                    : m.variant_dark_named({ name: selected.label })
                }
                onClick={() => {
                  setVariant(selected.themeIndex)
                  onUseMode(selected.theme, appearance)
                }}
              />
            </div>
            <span
              className='text-foreground pointer-events-none absolute bottom-0 left-1/2 inline-flex max-w-24 -translate-x-1/2 items-center gap-1 text-xs font-medium'
              style={{ marginLeft: offset }}
            >
              <span className='truncate'>{selected.label}</span>
              {options.length > 1 ? (
                <span className='bg-muted text-muted-foreground shrink-0 rounded-full px-1 text-[0.625rem]'>
                  +{options.length - 1}
                </span>
              ) : null}
            </span>
            {options.length > 1
              ? options.map((option, optionIndex) => {
                  const progress = optionIndex / (options.length - 1) - 0.5
                  const x = offset + progress * 68
                  const y = Math.abs(progress) * 10
                  const optionPicked = pickedModes(option.theme.id).includes(
                    appearance
                  )
                  return (
                    <button
                      key={option.theme.id}
                      type='button'
                      aria-pressed={optionPicked}
                      aria-label={
                        appearance === 'light'
                          ? m.use_variant_light({ name: option.label })
                          : m.use_variant_dark({ name: option.label })
                      }
                      tabIndex={open ? 0 : -1}
                      onMouseEnter={() => setOpenMode(appearance)}
                      onFocus={() => setOpenMode(appearance)}
                      onClick={(event) => {
                        event.stopPropagation()
                        setVariant(option.themeIndex)
                        onUseMode(option.theme, appearance)
                      }}
                      className={cn(
                        'bg-background focus-visible:ring-ring absolute top-1 left-1/2 z-30 flex size-7 items-center justify-center rounded-full shadow-sm transition-[opacity,transform] duration-200 ease-out outline-none focus-visible:ring-2 motion-reduce:transition-none',
                        optionPicked
                          ? 'ring-ring ring-2 ring-inset'
                          : 'ring-border/70 ring-1'
                      )}
                      style={{
                        opacity: open ? 1 : 0,
                        pointerEvents: open ? 'auto' : 'none',
                        transform: `translate(calc(-50% + ${open ? x : offset}px), ${open ? y : 28}px) scale(${open ? 1 : 0.55})`,
                        transitionDelay: open ? `${optionIndex * 35}ms` : '0ms'
                      }}
                    >
                      <ThemeOrb
                        colors={themeColorsFor(option.theme, appearance)!}
                        appearance={appearance}
                        className='size-6 border'
                      />
                    </button>
                  )
                })
              : null}
          </div>
        )
      })}
    </fieldset>
  )

  return (
    <ThemeCard
      theme={theme}
      name={collectionLabel}
      pickedModes={pickedModes(theme.id)}
      orbs={orbs}
      tooltip={m.use_first_variants()}
      removeLabel={m.remove_themes()}
      onUse={useDefaults}
      onUseMode={(appearance) => onUseMode(theme, appearance)}
      onDuplicate={() => onDuplicate(theme)}
      onEdit={() => onEdit(theme)}
      onExport={() => onExport(theme)}
      onRemove={() => onRemove(theme)}
    />
  )
}

function ThemeOrbs({
  theme,
  modes,
  pickedModes,
  onUseMode
}: {
  theme: ThemeDefinition
  modes: readonly ThemeAppearance[]
  pickedModes: readonly ThemeAppearance[]
  onUseMode: (appearance: ThemeAppearance) => void
}) {
  return (
    <div className='flex min-h-16 items-center justify-center gap-2.5 px-3 pt-3'>
      {modes.map((appearance) => (
        <OrbButton
          key={appearance}
          theme={theme}
          appearance={appearance}
          picked={pickedModes.includes(appearance)}
          onClick={() => onUseMode(appearance)}
        />
      ))}
    </div>
  )
}

export function OrbButton({
  theme,
  appearance,
  picked,
  onClick,
  label,
  tooltip,
  onHover
}: {
  theme: ThemeDefinition
  appearance: ThemeAppearance
  picked: boolean
  onClick: () => void
  label?: string
  tooltip?: string
  onHover?: () => void
}) {
  const colors = themeColorsFor(theme, appearance) ?? theme.colors
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type='button'
            aria-pressed={picked}
            aria-label={
              label ??
              (appearance === 'light'
                ? m.use_theme_light_named({ name: theme.label })
                : m.use_theme_dark_named({ name: theme.label }))
            }
            onClick={(event) => {
              event.stopPropagation()
              onClick()
            }}
            onMouseEnter={onHover}
            onFocus={onHover}
            className={cn(
              'focus-visible:outline-foreground/60 relative flex size-17 shrink-0 cursor-pointer items-center justify-center rounded-full p-1.5 transition-transform outline-none focus-visible:outline-2 focus-visible:-outline-offset-4',
              picked ? 'hover:scale-100' : 'hover:scale-105'
            )}
          />
        }
      >
        <ThemeOrb colors={colors} appearance={appearance} />
        {picked ? (
          <>
            <span
              aria-hidden
              className='ring-ring pointer-events-none absolute inset-0 rounded-full ring-2 ring-inset'
            />
            <span
              aria-hidden
              className='border-border/70 bg-background text-foreground pointer-events-none absolute right-0.5 bottom-0.5 flex size-5 items-center justify-center rounded-full border shadow-sm'
            >
              <HugeiconsIcon
                icon={appearance === 'light' ? Sun03Icon : Moon02Icon}
                strokeWidth={2}
                className='size-3'
              />
            </span>
          </>
        ) : null}
      </TooltipTrigger>
      <TooltipContent>
        {tooltip ??
          (appearance === 'light'
            ? m.use_theme_light_only()
            : m.use_theme_dark_only())}
      </TooltipContent>
    </Tooltip>
  )
}

function CardAction({
  label,
  icon,
  onClick,
  destructive = false
}: {
  label: string
  icon: typeof Copy01Icon
  onClick: () => void
  destructive?: boolean
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant='ghost'
            size='icon-xs'
            aria-label={label}
            onClick={(event) => {
              event.stopPropagation()
              onClick()
            }}
            className={cn(
              'text-muted-foreground',
              destructive && 'hover:text-destructive'
            )}
          />
        }
      >
        <HugeiconsIcon icon={icon} strokeWidth={2} />
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}

export function CreateThemeButton() {
  const openThemeEditor = useThemeEditorStore((state) => state.openThemeEditor)
  const customThemes = useThemeStore((state) => state.customThemes)
  const selection = useSelection()
  const initialAppearance = useResolvedAppearance()
  return (
    <Button
      size='xs'
      variant='outline'
      onClick={() =>
        openThemeEditor({
          editingThemeId: null,
          seedThemeId: halfOwners(selection, customThemes)[initialAppearance],
          seedName: null,
          initialAppearance
        })
      }
    >
      <HugeiconsIcon icon={PaintBrush01Icon} strokeWidth={2} />
      {m.create_theme()}
    </Button>
  )
}

export function AddThemeButton() {
  const [open, setOpen] = useState(false)
  return (
    <>
      <Button size='xs' variant='outline' onClick={() => setOpen(true)}>
        <HugeiconsIcon icon={Add01Icon} strokeWidth={2} />
        {m.add_theme()}
      </Button>
      <ThemeImportDialog
        open={open}
        onOpenChange={setOpen}
        chooseImported={(theme, selection) => chooseCard(selection, theme)}
      />
    </>
  )
}
