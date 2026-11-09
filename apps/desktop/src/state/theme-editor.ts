import { isColor } from '@/lib/color'
import {
  chooseTheme,
  deriveThemeColors,
  installTheme,
  otherAppearance,
  parseThemeFile,
  removeThemes,
  STANDARD_THEME_ID,
  standardColors,
  THEME_FILE_VERSION,
  type ThemeAppearance,
  type ThemeColorRole,
  type ThemeColors,
  type ThemeDefinition,
  ThemeError,
  type ThemeSelection,
  themeColorsFor,
  themeIdFromName,
  themeModes,
  updateTheme
} from './themes'

/** The two colors the simple mode edits; every other role derives from them. */
export const SIMPLE_ROLES = [
  'background',
  'primary'
] as const satisfies readonly ThemeColorRole[]

export type EditorColors = Record<ThemeAppearance, ThemeColors>

export type EditorStart = {
  colors: EditorColors
  appearance: ThemeAppearance
  advanced: boolean
  /** Leaving the advanced mode rebuilds the colors from the two simple ones. */
  regenerate: boolean
}

/** Where the editor opens: the edited or seeded theme's colors, else hoshi's own. */
export function startEditor(
  source: ThemeDefinition | null,
  initialAppearance: ThemeAppearance
): EditorStart {
  const seed = source?.id === STANDARD_THEME_ID ? null : source
  const colors: EditorColors = {
    light: standardColors('light'),
    dark: standardColors('dark')
  }
  if (seed) {
    for (const appearance of themeModes(seed)) {
      colors[appearance] = { ...themeColorsFor(seed, appearance)! }
    }
  }
  const appearance =
    seed && !themeColorsFor(seed, initialAppearance)
      ? seed.appearance
      : initialAppearance
  const advanced = seed !== null && seed.managed !== true
  return { colors, appearance, advanced, regenerate: advanced }
}

/** The colors the simple mode derives for `appearance` from its background and accent. */
export function managedColors(
  appearance: ThemeAppearance,
  colors: ThemeColors
): ThemeColors {
  const defaults = standardColors(appearance)
  return deriveThemeColors(
    appearance,
    isColor(colors.background.trim()) ? colors.background : defaults.background,
    isColor(colors.primary.trim()) ? colors.primary : defaults.primary
  )
}

/** A custom theme the name already belongs to, so saving adds a palette to it. */
export function findMergeTarget(
  name: string,
  customThemes: readonly ThemeDefinition[],
  editingId: string | undefined
): ThemeDefinition | null {
  const normalized = name.trim().toLowerCase()
  if (!normalized) return null
  const id = themeIdFromName(name)
  return (
    customThemes.find(
      (theme) =>
        theme.id !== editingId &&
        (theme.id === id || theme.label.trim().toLowerCase() === normalized)
    ) ?? null
  )
}

export type AppearanceLock =
  | { reason: 'editing_lacks'; name: string; appearance: ThemeAppearance }
  | { reason: 'merge_has'; name: string; appearance: ThemeAppearance }

/** Why the editor can't switch to `appearance`, or `null` when it can. */
export function appearanceLock(
  appearance: ThemeAppearance,
  editing: ThemeDefinition | null,
  mergeTarget: ThemeDefinition | null
): AppearanceLock | null {
  if (editing && !themeModes(editing).includes(appearance)) {
    return { reason: 'editing_lacks', name: editing.label, appearance }
  }
  if (!editing && mergeTarget && themeModes(mergeTarget).includes(appearance)) {
    return { reason: 'merge_has', name: mergeTarget.label, appearance }
  }
  return null
}

export type EditorSave = {
  name: string
  editing: ThemeDefinition | null
  appearance: ThemeAppearance
  colors: EditorColors
  advanced: boolean
  /** The simple colors changed per appearance, so the derived ones must be rebuilt. */
  simpleDirty: Record<ThemeAppearance, boolean>
  customThemes: readonly ThemeDefinition[]
}

export type EditorResult = {
  themes: ThemeDefinition[]
  saved: ThemeDefinition
  created: boolean
  mergedAppearance?: ThemeAppearance
  /** What the app shows once saved; `null` leaves the choice as it was. */
  selection: ThemeSelection | null
}

function themeFile(
  theme: {
    id?: string
    name: string
    appearance: ThemeAppearance
    colors: ThemeColors
    variants?: Partial<Record<ThemeAppearance, ThemeColors>>
    managed: boolean
  },
  collection: ThemeDefinition['collection']
): ThemeDefinition {
  const parsed = parseThemeFile({
    version: THEME_FILE_VERSION,
    ...(theme.id ? { id: theme.id } : {}),
    name: theme.name,
    appearance: theme.appearance,
    colors: theme.colors,
    ...(theme.variants && Object.keys(theme.variants).length > 0
      ? { variants: theme.variants }
      : {}),
    ...(theme.managed ? { managed: true } : {})
  })
  return collection ? { ...parsed, collection } : parsed
}

/** Saves what the editor holds: a new theme, an edit, or a palette added to a same-named theme. */
export function saveEditor(input: EditorSave): EditorResult {
  const { name, editing, appearance, colors, advanced, customThemes } = input
  if (!name.trim()) throw new ThemeError('missing_name')
  const forSave: EditorColors = advanced
    ? colors
    : {
        light: input.simpleDirty.light
          ? managedColors('light', colors.light)
          : colors.light,
        dark: input.simpleDirty.dark
          ? managedColors('dark', colors.dark)
          : colors.dark
      }
  const mergeTarget = findMergeTarget(name, customThemes, editing?.id)
  const taken = mergeTarget ? themeModes(mergeTarget) : []
  const mergedVariants = (
    target: ThemeDefinition,
    modes: ThemeAppearance[]
  ) => {
    const variants: Partial<Record<ThemeAppearance, ThemeColors>> = {
      ...target.variants
    }
    for (const mode of modes) variants[mode] = forSave[mode]
    return variants
  }

  if (editing && mergeTarget) {
    const edited = themeModes(editing)
    const collision = edited.find((mode) => taken.includes(mode))
    if (collision) {
      throw new ThemeError('palette_taken', mergeTarget.label, collision)
    }
    const saved = themeFile(
      {
        id: mergeTarget.id,
        name: mergeTarget.label,
        appearance: mergeTarget.appearance,
        colors: mergeTarget.colors,
        variants: mergedVariants(mergeTarget, edited),
        managed: mergeTarget.managed === true && !advanced
      },
      mergeTarget.collection
    )
    return {
      themes: removeThemes(updateTheme(customThemes, saved), [editing.id]),
      saved,
      created: false,
      mergedAppearance: edited[0]!,
      selection: chooseTheme(saved.id)
    }
  }

  if (editing) {
    const base = editing.appearance
    const variant = otherAppearance(base)
    const saved = themeFile(
      {
        id: editing.id,
        name,
        appearance: base,
        colors: forSave[base],
        ...(themeModes(editing).length > 1
          ? { variants: { [variant]: forSave[variant] } }
          : {}),
        managed: !advanced
      },
      editing.collection
    )
    return {
      themes: updateTheme(customThemes, saved),
      saved,
      created: false,
      selection: null
    }
  }

  if (mergeTarget) {
    if (taken.includes(appearance)) {
      throw new ThemeError('palettes_taken', mergeTarget.label)
    }
    const saved = themeFile(
      {
        id: mergeTarget.id,
        name: mergeTarget.label,
        appearance: mergeTarget.appearance,
        colors: mergeTarget.colors,
        variants: mergedVariants(mergeTarget, [appearance]),
        managed: mergeTarget.managed === true && !advanced
      },
      mergeTarget.collection
    )
    return {
      themes: updateTheme(customThemes, saved),
      saved,
      created: false,
      mergedAppearance: appearance,
      selection: chooseTheme(saved.id)
    }
  }

  const saved = themeFile(
    { name, appearance, colors: forSave[appearance], managed: !advanced },
    undefined
  )
  return {
    themes: installTheme(customThemes, saved),
    saved,
    created: true,
    selection: chooseTheme(saved.id)
  }
}
