import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { HugeiconsIcon } from '@hugeicons/react'
import { confirm, notify } from '@/state/dialog-store'
import { Button } from '@hoshi/ui/components/button'
import { useThemeEditorStore } from '@/state/theme-editor-store'
import { useResolvedAppearance, useThemeStore } from '@/state/theme-store'
import { THEME_OPTIONS } from './theme-options'
import type { ThemePreference } from '@/state/appearance'
import { ThemeOrb, ThemeWireframe } from './ThemePreviews'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'
import {
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

export function ThemeGrid() {
  const selection = useSelection()
  const customThemes = useThemeStore((state) => state.customThemes)
  const setSelection = useThemeStore((state) => state.setSelection)
  const setCustomThemes = useThemeStore((state) => state.setCustomThemes)
  const openThemeEditor = useThemeEditorStore((state) => state.openThemeEditor)
  const initialAppearance = useResolvedAppearance()
  const owners = halfOwners(selection, customThemes)
  const pickedModes = (cardId: string | null) =>
    (['light', 'dark'] as const).filter(
      (appearance) => owners[appearance] === cardId
    )

  const duplicate = (theme: ThemeDefinition) =>
    openThemeEditor({
      editingThemeId: null,
      seedThemeId: theme.id === STANDARD_THEME_ID ? null : theme.id,
      seedName: m.theme_copy_name({ name: theme.label }),
      initialAppearance
    })

  const exportTheme = (theme: ThemeDefinition) => {
    invoke<boolean>('export_theme', {
      fileName: `${theme.id}.json`,
      contents: serializeThemeFile(theme)
    }).catch(() =>
      notify({
        title: m.export_theme_failed(),
        confirmLabel: m.ok(),
        tone: 'warning'
      })
    )
  }

  const remove = async (theme: ThemeDefinition) => {
    const confirmed = await confirm({
      title: m.remove_theme_confirm({ name: theme.label }),
      description: m.remove_theme_description(),
      confirmLabel: m.remove_theme(),
      tone: 'destructive'
    })
    if (!confirmed) return
    const {
      customThemes: current,
      theme: base,
      halves
    } = useThemeStore.getState()
    setCustomThemes(
      removeThemes(current, [theme.id]),
      selectionWithout({ theme: base, halves }, [theme.id])
    )
  }

  return (
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
      {customThemes.map((theme) => (
        <ThemeCard
          key={theme.id}
          theme={theme}
          pickedModes={pickedModes(theme.id)}
          onUse={() => setSelection(chooseCard(selection, theme))}
          onUseMode={(appearance) =>
            setSelection(chooseHalf(selection, appearance, theme.id))
          }
          onDuplicate={() => duplicate(theme)}
          onEdit={() =>
            openThemeEditor({
              editingThemeId: theme.id,
              seedThemeId: null,
              seedName: null,
              initialAppearance
            })
          }
          onExport={() => exportTheme(theme)}
          onRemove={() => void remove(theme)}
        />
      ))}
    </div>
  )
}

function ThemeCard({
  theme,
  pickedModes,
  onUse,
  onUseMode,
  onDuplicate,
  onEdit,
  onExport,
  onRemove
}: {
  theme: ThemeDefinition
  pickedModes: readonly ThemeAppearance[]
  onUse: () => void
  onUseMode: (appearance: ThemeAppearance) => void
  onDuplicate: () => void
  onEdit?: () => void
  onExport?: () => void
  onRemove?: () => void
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
        <ThemeOrbs
          theme={theme}
          modes={modes}
          pickedModes={pickedModes}
          onUseMode={onUseMode}
        />
        <div className='flex items-center gap-2 px-3 pt-2 pb-2.5'>
          <button
            type='button'
            aria-pressed={pickedModes.length > 0}
            aria-label={m.use_theme_named({ name: theme.label })}
            onClick={(event) => {
              event.stopPropagation()
              onUse()
            }}
            className='focus-visible:ring-ring min-w-0 flex-1 cursor-pointer truncate rounded-sm text-left text-sm font-medium outline-none focus-visible:ring-2'
          >
            {theme.label}
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
                label={m.remove_theme()}
                icon={Delete02Icon}
                onClick={onRemove}
                destructive
              />
            ) : null}
          </div>
        </div>
      </TooltipTrigger>
      <TooltipContent>
        {modes.length > 1
          ? m.use_theme_both()
          : modes[0] === 'light'
            ? m.use_theme_light_only()
            : m.use_theme_dark_only()}
      </TooltipContent>
    </Tooltip>
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
  tooltip
}: {
  theme: ThemeDefinition
  appearance: ThemeAppearance
  picked: boolean
  onClick: () => void
  label?: string
  tooltip?: string
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
