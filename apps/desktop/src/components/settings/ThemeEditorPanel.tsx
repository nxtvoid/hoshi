import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { Input } from '@hoshi/ui/components/input'
import { Button } from '@hoshi/ui/components/button'
import { Switch } from '@hoshi/ui/components/switch'
import { themeErrorMessage } from './theme-error-messages'
import { previewColor, ThemeColorField } from './ThemeColorField'
import {
  type PointerEvent as ReactPointerEvent,
  useEffect,
  useId,
  useRef,
  useState
} from 'react'
import { setThemePreview, useThemeStore } from '@/state/theme-store'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'
import {
  Add01Icon,
  ArrowDown01Icon,
  ArrowUp01Icon,
  Cancel01Icon,
  PaintBrush01Icon
} from '@hugeicons/core-free-icons'
import {
  type AppearanceLock,
  appearanceLock,
  type EditorColors,
  findMergeTarget,
  managedColors,
  SIMPLE_ROLES,
  saveEditor,
  startEditor
} from '@/state/theme-editor'
import {
  type ThemeEditorSession,
  useThemeEditorStore
} from '@/state/theme-editor-store'
import {
  findTheme,
  standardColors,
  THEME_APPEARANCES,
  THEME_COLOR_ROLES,
  type ThemeAppearance,
  type ThemeColorRole,
  type ThemeColors,
  ThemeError,
  themeModes,
  updateColorFamily
} from '@/state/themes'

type Family = { role: ThemeColorRole; label: () => string }

const GROUPS: readonly { title: () => string; families: Family[] }[] = [
  {
    title: m.theme_group_foundation,
    families: [
      { role: 'background', label: m.theme_color_background },
      { role: 'card', label: m.theme_color_surface },
      { role: 'foreground', label: m.theme_color_text },
      { role: 'mutedForeground', label: m.theme_color_muted_text },
      { role: 'border', label: m.theme_color_border },
      { role: 'input', label: m.theme_color_input }
    ]
  },
  {
    title: m.theme_group_brand,
    families: [
      { role: 'primary', label: m.theme_color_accent },
      { role: 'secondary', label: m.theme_color_subtle_surface },
      { role: 'accent', label: m.theme_color_highlight_surface },
      { role: 'ring', label: m.theme_color_focus },
      { role: 'link', label: m.theme_color_link },
      { role: 'header', label: m.theme_color_header }
    ]
  },
  {
    title: m.theme_group_context,
    families: [
      { role: 'sidebar', label: m.theme_color_sidebar },
      { role: 'sidebarAccent', label: m.theme_color_sidebar_selection },
      { role: 'terminalSelection', label: m.theme_color_terminal_selection },
      { role: 'scrollbar', label: m.theme_color_scrollbar }
    ]
  },
  {
    title: m.theme_group_status,
    families: [
      { role: 'destructive', label: m.theme_color_error },
      { role: 'statusWaiting', label: m.theme_color_warning }
    ]
  }
]

const SIMPLE_LABELS: Record<(typeof SIMPLE_ROLES)[number], () => string> = {
  background: m.theme_color_background,
  primary: m.theme_color_accent
}

const PANEL_MARGIN = 8
const PANEL_MIN_WIDTH = 280
const PANEL_MIN_HEIGHT = 220

function clampPosition(x: number, y: number, width: number) {
  const maxX = Math.max(PANEL_MARGIN, window.innerWidth - width - PANEL_MARGIN)
  const maxY = Math.max(PANEL_MARGIN, window.innerHeight - 48)
  return {
    x: Math.min(Math.max(x, PANEL_MARGIN), maxX),
    y: Math.min(Math.max(y, PANEL_MARGIN), maxY)
  }
}

function lockMessage(lock: AppearanceLock): string {
  if (lock.reason === 'editing_lacks') {
    return lock.appearance === 'light'
      ? m.theme_lacks_light({ name: lock.name })
      : m.theme_lacks_dark({ name: lock.name })
  }
  return lock.appearance === 'light'
    ? m.theme_has_light({ name: lock.name })
    : m.theme_has_dark({ name: lock.name })
}

function previewColors(colors: ThemeColors, appearance: ThemeAppearance) {
  const fallback = standardColors(appearance)
  const preview = {} as Record<ThemeColorRole, string>
  for (const role of THEME_COLOR_ROLES) {
    preview[role] = previewColor(colors[role], fallback[role])
  }
  return preview
}

/** Mounts the editor while a session is open; Settings hosts it so it outlives a section switch. */
export function ThemeEditorHost() {
  const session = useThemeEditorStore((state) => state.session)
  const close = useThemeEditorStore((state) => state.closeThemeEditor)
  useEffect(() => close, [close])
  if (!session) return null
  return <ThemeEditorPanel key={session.id} session={session} onClose={close} />
}

function ThemeEditorPanel({
  session,
  onClose
}: {
  session: ThemeEditorSession
  onClose: () => void
}) {
  const customThemes = useThemeStore((state) => state.customThemes)
  const setCustomThemes = useThemeStore((state) => state.setCustomThemes)
  const [editing] = useState(() =>
    session.editingThemeId
      ? (customThemes.find((theme) => theme.id === session.editingThemeId) ??
        null)
      : null
  )
  const [start] = useState(() =>
    startEditor(
      editing ??
        (session.seedThemeId
          ? findTheme(session.seedThemeId, customThemes)
          : null),
      session.initialAppearance
    )
  )
  const [name, setName] = useState(editing?.label ?? session.seedName ?? '')
  const [appearance, setAppearance] = useState(start.appearance)
  const [advanced, setAdvanced] = useState(start.advanced)
  const [colors, setColors] = useState<EditorColors>(start.colors)
  const [simpleDirty, setSimpleDirty] = useState({ light: false, dark: false })
  const [regenerate, setRegenerate] = useState(start.regenerate)
  const [error, setError] = useState<string | null>(null)
  const [minimized, setMinimized] = useState(false)
  const [query, setQuery] = useState('')
  const [position, setPosition] = useState<{
    x: number
    y: number
    width: number
  } | null>(null)
  const [size, setSize] = useState<{ width: number; height: number } | null>(
    null
  )
  const panelRef = useRef<HTMLDivElement>(null)
  const id = useId()
  const snapshot = useRef<{
    colors: EditorColors
    simpleDirty: Record<ThemeAppearance, boolean>
    regenerate: boolean
  } | null>(null)
  const dragOffset = useRef<{ dx: number; dy: number; width: number } | null>(
    null
  )
  const resizeStart = useRef<{
    x: number
    y: number
    left: number
    top: number
    width: number
    height: number
  } | null>(null)

  const mergeTarget = findMergeTarget(name, customThemes, editing?.id)
  const takenModes = mergeTarget ? themeModes(mergeTarget) : []
  const lockFor = (target: ThemeAppearance) =>
    appearanceLock(target, editing, mergeTarget)

  const mergeKey =
    editing || !mergeTarget ? null : `${mergeTarget.id}:${takenModes.join(',')}`
  const [appliedMergeKey, setAppliedMergeKey] = useState<string | null>(null)
  if (appliedMergeKey !== mergeKey) {
    setAppliedMergeKey(mergeKey)
    if (
      mergeKey !== null &&
      takenModes.length === 1 &&
      takenModes.includes(appearance)
    ) {
      setAppearance(takenModes[0] === 'light' ? 'dark' : 'light')
    }
  }

  useEffect(() => {
    setThemePreview({
      colors: previewColors(colors[appearance], appearance),
      appearance
    })
  }, [appearance, colors])

  useEffect(() => () => setThemePreview(null), [])

  const updateColor = (role: ThemeColorRole, value: string) => {
    const simple = (SIMPLE_ROLES as readonly ThemeColorRole[]).includes(role)
    setColors((current) => {
      const next = { ...current[appearance], [role]: value }
      return {
        ...current,
        [appearance]: advanced
          ? updateColorFamily(current[appearance], role, value)
          : simple
            ? managedColors(appearance, next)
            : next
      }
    })
    if (!advanced && simple) {
      setSimpleDirty((current) => ({ ...current, [appearance]: true }))
    }
    if (advanced) setRegenerate(true)
  }

  const startColorEdit = () => {
    snapshot.current = { colors, simpleDirty, regenerate }
  }

  const cancelColorEdit = () => {
    const saved = snapshot.current
    snapshot.current = null
    if (!saved) return
    setColors(saved.colors)
    setSimpleDirty(saved.simpleDirty)
    setRegenerate(saved.regenerate)
  }

  const changeAdvanced = (checked: boolean) => {
    setAdvanced(checked)
    if (checked || !regenerate) return
    const modes: ThemeAppearance[] =
      editing && themeModes(editing).length > 1
        ? ['light', 'dark']
        : [appearance]
    setSimpleDirty((current) => {
      const next = { ...current }
      for (const mode of modes) next[mode] = true
      return next
    })
    setColors((current) => {
      const next = { ...current }
      for (const mode of modes) next[mode] = managedColors(mode, current[mode])
      return next
    })
    setRegenerate(false)
  }

  const submit = () => {
    try {
      const result = saveEditor({
        name,
        editing,
        appearance,
        colors,
        advanced,
        simpleDirty,
        customThemes: useThemeStore.getState().customThemes
      })
      setCustomThemes(result.themes, result.selection ?? undefined)
      onClose()
    } catch (cause) {
      setError(
        cause instanceof ThemeError
          ? themeErrorMessage(cause)
          : editing
            ? m.theme_save_failed()
            : m.theme_create_failed()
      )
    }
  }

  const handleDragDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if ((event.target as HTMLElement).closest('button, input, a')) return
    const rect = panelRef.current?.getBoundingClientRect()
    if (!rect) return
    dragOffset.current = {
      dx: event.clientX - rect.x,
      dy: event.clientY - rect.y,
      width: rect.width
    }
    event.currentTarget.setPointerCapture(event.pointerId)
  }

  const handleDragMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const offset = dragOffset.current
    if (!offset) return
    const next = clampPosition(
      event.clientX - offset.dx,
      event.clientY - offset.dy,
      offset.width
    )
    setPosition({ ...next, width: offset.width })
  }

  const handleResizeDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    const rect = panelRef.current?.getBoundingClientRect()
    if (!rect) return
    event.preventDefault()
    setPosition({
      ...clampPosition(rect.x, rect.y, rect.width),
      width: rect.width
    })
    resizeStart.current = {
      x: event.clientX,
      y: event.clientY,
      left: rect.x,
      top: rect.y,
      width: rect.width,
      height: rect.height
    }
    event.currentTarget.setPointerCapture(event.pointerId)
  }

  const handleResizeMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const from = resizeStart.current
    if (!from) return
    const maxWidth = Math.max(
      PANEL_MIN_WIDTH,
      window.innerWidth - PANEL_MARGIN - from.left
    )
    const maxHeight = Math.max(
      PANEL_MIN_HEIGHT,
      window.innerHeight - PANEL_MARGIN - from.top
    )
    setSize({
      width: Math.min(
        Math.max(from.width + event.clientX - from.x, PANEL_MIN_WIDTH),
        maxWidth
      ),
      height: Math.min(
        Math.max(from.height + event.clientY - from.y, PANEL_MIN_HEIGHT),
        maxHeight
      )
    })
  }

  const shownSize = size && {
    width: Math.max(
      PANEL_MIN_WIDTH,
      Math.min(size.width, window.innerWidth - PANEL_MARGIN * 2)
    ),
    height: Math.max(
      PANEL_MIN_HEIGHT,
      Math.min(size.height, window.innerHeight - PANEL_MARGIN * 2)
    )
  }
  const shownPosition =
    position &&
    clampPosition(position.x, position.y, shownSize?.width ?? position.width)

  const filter = query.trim().toLowerCase()
  const groups = GROUPS.map((group) => ({
    title: group.title,
    families: group.families.filter(
      (family) => !filter || family.label().toLowerCase().includes(filter)
    )
  })).filter((group) => group.families.length > 0)

  const field = (role: ThemeColorRole, label: string) => (
    <ThemeColorField
      key={role}
      label={label}
      value={colors[appearance][role]}
      onChange={(value) => {
        setError(null)
        updateColor(role, value)
      }}
      onEditStart={startColorEdit}
      onEditCancel={cancelColorEdit}
    />
  )

  const title = editing ? m.edit_theme() : m.create_theme()

  return (
    <div
      ref={panelRef}
      role='dialog'
      aria-label={title}
      className={cn(
        'bg-popover text-popover-foreground ring-foreground/10 fixed z-60 flex max-h-[min(42rem,calc(100dvh-6rem))] w-[min(26rem,calc(100vw-2rem))] flex-col overflow-hidden rounded-xl shadow-2xl ring-1',
        position === null && 'right-4 bottom-4',
        minimized && 'max-h-none'
      )}
      style={{
        ...(shownPosition
          ? { left: shownPosition.x, top: shownPosition.y }
          : {}),
        ...(shownSize ? { width: shownSize.width } : {}),
        ...(shownSize && !minimized
          ? { height: shownSize.height, maxHeight: 'calc(100dvh - 1rem)' }
          : {})
      }}
    >
      <div
        className='border-border/70 flex cursor-grab touch-none items-center gap-1 border-b px-3 py-2 select-none active:cursor-grabbing'
        onPointerDown={handleDragDown}
        onPointerMove={handleDragMove}
        onPointerUp={() => {
          dragOffset.current = null
        }}
        onPointerCancel={() => {
          dragOffset.current = null
        }}
      >
        <h2 className='min-w-0 flex-1 truncate text-sm font-medium'>{title}</h2>
        <PanelButton
          label={
            minimized ? m.expand_theme_editor() : m.minimize_theme_editor()
          }
          icon={minimized ? ArrowUp01Icon : ArrowDown01Icon}
          onClick={() => setMinimized(!minimized)}
        />
        <PanelButton
          label={m.close_theme_editor()}
          icon={Cancel01Icon}
          onClick={onClose}
        />
      </div>
      {minimized ? null : (
        <>
          <div className='flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto py-3 ps-3 pe-1.5'>
            <div className='grid grid-cols-[minmax(0,1fr)_minmax(0,2fr)] items-center gap-3'>
              <label htmlFor={`${id}-name`} className='text-sm font-medium'>
                {m.theme_name()}
              </label>
              <Input
                id={`${id}-name`}
                autoFocus
                className='h-7'
                value={name}
                placeholder={
                  editing ? m.theme_name() : m.theme_name_placeholder()
                }
                onChange={(event) => {
                  setName(event.currentTarget.value)
                  setError(null)
                }}
              />
            </div>
            {error ? (
              <p aria-live='polite' className='text-destructive text-sm'>
                {error}
              </p>
            ) : null}
            <div className='grid grid-cols-[minmax(0,1fr)_minmax(0,2fr)] items-center gap-3'>
              <span className='text-sm font-medium'>
                {m.theme_appearance()}
              </span>
              <fieldset className='bg-muted grid grid-cols-2 gap-0.5 rounded-lg p-0.5'>
                <legend className='sr-only'>{m.theme_appearance()}</legend>
                {THEME_APPEARANCES.map((option) => {
                  const lock = lockFor(option)
                  const button = (
                    <button
                      type='button'
                      aria-pressed={appearance === option}
                      aria-disabled={lock !== null}
                      onClick={() => {
                        if (lock === null) setAppearance(option)
                      }}
                      className={cn(
                        'focus-visible:ring-ring h-6 rounded-md text-xs font-medium transition-colors outline-none focus-visible:ring-2',
                        appearance === option
                          ? 'bg-background text-foreground shadow-sm'
                          : 'text-muted-foreground hover:text-foreground',
                        lock !== null && 'cursor-not-allowed opacity-50'
                      )}
                    >
                      {option === 'light' ? m.theme_light() : m.theme_dark()}
                    </button>
                  )
                  return lock === null ? (
                    <span key={option} className='contents'>
                      {button}
                    </span>
                  ) : (
                    <Tooltip key={option}>
                      <TooltipTrigger render={button} />
                      <TooltipContent>{lockMessage(lock)}</TooltipContent>
                    </Tooltip>
                  )
                })}
              </fieldset>
            </div>
            <div className='flex flex-col gap-3'>
              <div className='grid grid-cols-[minmax(0,1fr)_minmax(0,2fr)] items-start gap-3'>
                <div>
                  <h3 className='text-sm font-medium'>{m.theme_colors()}</h3>
                  {advanced ? null : (
                    <p className='text-muted-foreground text-xs'>
                      {m.theme_colors_simple_hint()}
                    </p>
                  )}
                </div>
                <div className='flex min-w-0 items-start gap-3'>
                  {advanced ? (
                    <Input
                      aria-label={m.filter_colors()}
                      placeholder={m.filter_colors()}
                      className='h-7 min-w-0 flex-1'
                      value={query}
                      onChange={(event) => setQuery(event.currentTarget.value)}
                    />
                  ) : null}
                  <div className='ms-auto flex shrink-0 items-center gap-2 pt-0.5'>
                    <label
                      htmlFor={`${id}-advanced`}
                      className='cursor-pointer text-sm font-medium'
                    >
                      {m.advanced()}
                    </label>
                    <Switch
                      id={`${id}-advanced`}
                      aria-label={m.use_advanced_theme_colors()}
                      checked={advanced}
                      onCheckedChange={(checked) => changeAdvanced(checked)}
                    />
                  </div>
                </div>
              </div>
              {advanced ? (
                <div className='flex flex-col gap-4'>
                  {groups.map((group) => (
                    <section
                      key={group.title()}
                      className='flex flex-col gap-1'
                    >
                      <h4 className='px-2 text-sm font-medium'>
                        {group.title()}
                      </h4>
                      {group.families.map((family) =>
                        field(family.role, family.label())
                      )}
                    </section>
                  ))}
                  {groups.length === 0 ? (
                    <p className='text-muted-foreground px-2 text-xs'>
                      {m.no_matches()}
                    </p>
                  ) : null}
                </div>
              ) : (
                <div className='flex flex-col gap-1'>
                  {SIMPLE_ROLES.map((role) =>
                    field(role, SIMPLE_LABELS[role]())
                  )}
                </div>
              )}
            </div>
          </div>
          <div className='border-border/70 flex items-center justify-end gap-2 border-t px-3 py-2'>
            <Button size='sm' variant='ghost' onClick={onClose}>
              {m.cancel()}
            </Button>
            <Button size='sm' disabled={!name.trim()} onClick={submit}>
              {editing ? (
                mergeTarget ? (
                  m.merge_into_theme({ name: mergeTarget.label })
                ) : (
                  m.save_changes()
                )
              ) : mergeTarget ? (
                <>
                  <HugeiconsIcon icon={Add01Icon} strokeWidth={2} />
                  {appearance === 'light'
                    ? m.add_light_palette()
                    : m.add_dark_palette()}
                </>
              ) : (
                <>
                  <HugeiconsIcon icon={PaintBrush01Icon} strokeWidth={2} />
                  {m.create_theme()}
                </>
              )}
            </Button>
          </div>
          <div
            aria-hidden
            className='text-muted-foreground/70 absolute right-0 bottom-0 z-10 flex size-5 cursor-se-resize touch-none items-end justify-end p-1 select-none'
            onPointerDown={handleResizeDown}
            onPointerMove={handleResizeMove}
            onPointerUp={() => {
              resizeStart.current = null
            }}
            onPointerCancel={() => {
              resizeStart.current = null
            }}
          >
            <svg
              className='size-2.5'
              fill='none'
              stroke='currentColor'
              strokeLinecap='round'
              strokeWidth='1.2'
              viewBox='0 0 8 8'
            >
              <path d='M7 1 1 7M7 4.5 4.5 7' />
            </svg>
          </div>
        </>
      )}
    </div>
  )
}

function PanelButton({
  label,
  icon,
  onClick
}: {
  label: string
  icon: typeof Cancel01Icon
  onClick: () => void
}) {
  return (
    <Button variant='ghost' size='icon-xs' aria-label={label} onClick={onClick}>
      <HugeiconsIcon icon={icon} strokeWidth={2} />
    </Button>
  )
}
