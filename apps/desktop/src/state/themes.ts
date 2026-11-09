import {
  BLACK,
  canonicalColor,
  colorToRgb,
  contrastRatio,
  isDarkRgb,
  mixRgb,
  type Oklch,
  oklchToColor,
  oklchToRgb,
  parseColor,
  type Rgb,
  readableForeground,
  readableOn,
  rgbToColor,
  rgbToOklch,
  softenedText,
  solveLightness,
  WHITE
} from '@/lib/color'

export const THEME_APPEARANCES = ['light', 'dark'] as const
export type ThemeAppearance = (typeof THEME_APPEARANCES)[number]

export const THEME_COLOR_ROLES = [
  'background',
  'foreground',
  'card',
  'cardForeground',
  'popover',
  'popoverForeground',
  'primary',
  'primaryForeground',
  'secondary',
  'secondaryForeground',
  'muted',
  'mutedForeground',
  'accent',
  'accentForeground',
  'destructive',
  'border',
  'input',
  'ring',
  'sidebar',
  'sidebarForeground',
  'sidebarPrimary',
  'sidebarPrimaryForeground',
  'sidebarAccent',
  'sidebarAccentForeground',
  'sidebarBorder',
  'sidebarRing',
  'header',
  'headerGlow',
  'link',
  'send',
  'statusError',
  'statusWaiting',
  'terminalSelection',
  'scrollbar',
  'scrollbarHover'
] as const
export type ThemeColorRole = (typeof THEME_COLOR_ROLES)[number]
export type ThemeColors = Readonly<Record<ThemeColorRole, string>>
export type ThemeVariants = Readonly<
  Partial<Record<ThemeAppearance, ThemeColors>>
>
export type ThemeCollection = Readonly<{ id: string; label: string }>

export type ThemeDefinition = Readonly<{
  id: string
  label: string
  appearance: ThemeAppearance
  colors: ThemeColors
  variants?: ThemeVariants
  collection?: ThemeCollection
  /** Made from a background and an accent, so the editor reopens in its simple mode. */
  managed?: boolean
}>

const ROLE_SET: ReadonlySet<string> = new Set(THEME_COLOR_ROLES)

/** The CSS variable a role sets, `sidebarBorder` → `--sidebar-border`. */
export function roleVariable(role: ThemeColorRole): string {
  return `--${role.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`)}`
}

export const STANDARD_THEME_ID = 'hoshi'
export const BUILT_IN_PALETTE_IDS = ['grove', 'ocean', 'ember', 'iris'] as const
export type BuiltInPaletteId = (typeof BUILT_IN_PALETTE_IDS)[number]

/** Custom themes can't take these, or they would capture a stored choice. */
export const RESERVED_THEME_IDS: ReadonlySet<string> = new Set([
  'system',
  'light',
  'dark',
  STANDARD_THEME_ID,
  ...BUILT_IN_PALETTE_IDS
])

export const THEME_NAME_MAX = 48
export const THEME_FILE_VERSION = 1

/** `packages/ui/src/styles/globals.css`, `:root`. */
const STANDARD_LIGHT: ThemeColors = {
  background: 'oklch(1 0 0)',
  foreground: 'oklch(0.145 0 0)',
  card: 'oklch(1 0 0)',
  cardForeground: 'oklch(0.145 0 0)',
  popover: 'oklch(1 0 0)',
  popoverForeground: 'oklch(0.145 0 0)',
  primary: 'oklch(0.205 0 0)',
  primaryForeground: 'oklch(0.985 0 0)',
  secondary: 'oklch(0.97 0 0)',
  secondaryForeground: 'oklch(0.205 0 0)',
  muted: 'oklch(0.97 0 0)',
  mutedForeground: 'oklch(0.556 0 0)',
  accent: 'oklch(0.97 0 0)',
  accentForeground: 'oklch(0.205 0 0)',
  destructive: 'oklch(0.577 0.245 27.325)',
  border: 'oklch(0.922 0 0)',
  input: 'oklch(0.922 0 0)',
  ring: 'oklch(0.708 0 0)',
  sidebar: 'oklch(0.985 0 0)',
  sidebarForeground: 'oklch(0.145 0 0)',
  sidebarPrimary: 'oklch(0.205 0 0)',
  sidebarPrimaryForeground: 'oklch(0.985 0 0)',
  sidebarAccent: 'oklch(0.97 0 0)',
  sidebarAccentForeground: 'oklch(0.205 0 0)',
  sidebarBorder: 'oklch(0.922 0 0)',
  sidebarRing: 'oklch(0.708 0 0)',
  header: 'oklch(0.975 0 0)',
  headerGlow: 'oklch(0.85 0 0 / 0.35)',
  link: 'oklch(0.55 0.17 252)',
  send: 'oklch(0.56 0.16 250)',
  statusError: 'oklch(0.6 0.21 25)',
  statusWaiting: 'oklch(0.72 0.16 70)',
  terminalSelection: 'oklch(0.708 0 0 / 0.35)',
  scrollbar: 'oklch(0.145 0 0 / 0.18)',
  scrollbarHover: 'oklch(0.145 0 0 / 0.34)'
}

/** `packages/ui/src/styles/globals.css`, `.dark`. */
const STANDARD_DARK: ThemeColors = {
  background: 'oklch(0.145 0 0)',
  foreground: 'oklch(0.985 0 0)',
  card: 'oklch(0.205 0 0)',
  cardForeground: 'oklch(0.985 0 0)',
  popover: 'oklch(0.205 0 0)',
  popoverForeground: 'oklch(0.985 0 0)',
  primary: 'oklch(0.922 0 0)',
  primaryForeground: 'oklch(0.205 0 0)',
  secondary: 'oklch(0.269 0 0)',
  secondaryForeground: 'oklch(0.985 0 0)',
  muted: 'oklch(0.269 0 0)',
  mutedForeground: 'oklch(0.708 0 0)',
  accent: 'oklch(0.269 0 0)',
  accentForeground: 'oklch(0.985 0 0)',
  destructive: 'oklch(0.704 0.191 22.216)',
  border: 'oklch(1 0 0 / 10%)',
  input: 'oklch(1 0 0 / 15%)',
  ring: 'oklch(0.556 0 0)',
  sidebar: 'oklch(0.205 0 0)',
  sidebarForeground: 'oklch(0.985 0 0)',
  sidebarPrimary: 'oklch(0.488 0.243 264.376)',
  sidebarPrimaryForeground: 'oklch(0.985 0 0)',
  sidebarAccent: 'oklch(0.269 0 0)',
  sidebarAccentForeground: 'oklch(0.985 0 0)',
  sidebarBorder: 'oklch(1 0 0 / 10%)',
  sidebarRing: 'oklch(0.556 0 0)',
  header: 'oklch(0.19 0 0)',
  headerGlow: 'oklch(0.85 0 0 / 0.08)',
  link: 'oklch(0.74 0.13 245)',
  send: 'oklch(0.74 0.13 245)',
  statusError: 'oklch(0.7 0.18 22)',
  statusWaiting: 'oklch(0.83 0.15 80)',
  terminalSelection: 'oklch(0.556 0 0 / 0.45)',
  scrollbar: 'oklch(0.985 0 0 / 0.16)',
  scrollbarHover: 'oklch(0.985 0 0 / 0.3)'
}

export function standardColors(appearance: ThemeAppearance): ThemeColors {
  return appearance === 'dark' ? STANDARD_DARK : STANDARD_LIGHT
}

/** `[data-palette]` in `globals.css`: the hue and chroma each palette tints with. */
export const PALETTE_TINTS: Readonly<
  Record<BuiltInPaletteId, { hue: number; chroma: number }>
> = {
  grove: { hue: 158, chroma: 0.11 },
  ocean: { hue: 245, chroma: 0.12 },
  ember: { hue: 48, chroma: 0.13 },
  iris: { hue: 294, chroma: 0.15 }
}

/** The colors `[data-palette]` and `.dark[data-palette]` in `globals.css` paint. */
export function paletteColors(
  palette: BuiltInPaletteId,
  appearance: ThemeAppearance
): ThemeColors {
  const { hue: h, chroma } = PALETTE_TINTS[palette]
  const at = (l: number, c: number, alpha?: number) =>
    alpha === undefined
      ? `oklch(${l} ${c} ${h})`
      : `oklch(${l} ${c} ${h} / ${alpha})`
  if (appearance === 'light') {
    return {
      ...STANDARD_LIGHT,
      background: at(0.988, 0.004),
      card: at(0.995, 0.003),
      popover: at(0.995, 0.003),
      primary: at(0.52, chroma),
      primaryForeground: at(0.985, 0.005),
      secondary: at(0.96, 0.01),
      secondaryForeground: at(0.24, 0.03),
      muted: at(0.96, 0.01),
      mutedForeground: at(0.53, 0.02),
      accent: at(0.95, 0.018),
      accentForeground: at(0.24, 0.03),
      border: at(0.915, 0.01),
      input: at(0.915, 0.01),
      ring: at(0.62, 0.1),
      sidebar: at(0.972, 0.01),
      sidebarPrimary: at(0.52, chroma),
      sidebarPrimaryForeground: at(0.985, 0.005),
      sidebarAccent: at(0.94, 0.02),
      sidebarAccentForeground: at(0.24, 0.03),
      sidebarBorder: at(0.915, 0.012),
      sidebarRing: at(0.62, 0.1),
      header: at(0.962, 0.012),
      headerGlow: at(0.85, 0.06, 0.35),
      terminalSelection: at(0.7, 0.08, 0.35)
    }
  }
  return {
    ...STANDARD_DARK,
    background: at(0.16, 0.01),
    card: at(0.215, 0.014),
    popover: at(0.215, 0.014),
    primary: at(0.78, chroma),
    primaryForeground: at(0.2, 0.03),
    secondary: at(0.27, 0.016),
    secondaryForeground: at(0.985, 0.004),
    muted: at(0.27, 0.016),
    mutedForeground: at(0.72, 0.015),
    accent: at(0.28, 0.022),
    accentForeground: at(0.985, 0.004),
    ring: at(0.58, 0.09),
    sidebar: at(0.2, 0.014),
    sidebarPrimary: at(0.78, chroma),
    sidebarPrimaryForeground: at(0.2, 0.03),
    sidebarAccent: at(0.28, 0.022),
    sidebarAccentForeground: at(0.985, 0.004),
    sidebarRing: at(0.58, 0.09),
    header: at(0.19, 0.012),
    headerGlow: at(0.85, 0.06, 0.08),
    terminalSelection: at(0.6, 0.08, 0.45)
  }
}

function builtInTheme(id: string, label: string): ThemeDefinition {
  const colors = (appearance: ThemeAppearance) =>
    id === STANDARD_THEME_ID
      ? standardColors(appearance)
      : paletteColors(id as BuiltInPaletteId, appearance)
  return {
    id,
    label,
    appearance: 'light',
    colors: colors('light'),
    variants: { dark: colors('dark') }
  }
}

/** Shipped themes; their labels are proper names, the same in every locale. */
export const BUILT_IN_THEMES: readonly ThemeDefinition[] = [
  builtInTheme(STANDARD_THEME_ID, 'Hoshi'),
  builtInTheme('grove', 'Grove'),
  builtInTheme('ocean', 'Ocean'),
  builtInTheme('ember', 'Ember'),
  builtInTheme('iris', 'Iris')
]

export function isBuiltInPalette(id: string): id is BuiltInPaletteId {
  return (BUILT_IN_PALETTE_IDS as readonly string[]).includes(id)
}

export function findTheme(
  id: string,
  customThemes: readonly ThemeDefinition[]
): ThemeDefinition | null {
  return (
    BUILT_IN_THEMES.find((theme) => theme.id === id) ??
    customThemes.find((theme) => theme.id === id) ??
    null
  )
}

export function themeColorsFor(
  theme: ThemeDefinition,
  appearance: ThemeAppearance
): ThemeColors | null {
  if (appearance === theme.appearance) return theme.colors
  return theme.variants?.[appearance] ?? null
}

export function themeModes(theme: ThemeDefinition): ThemeAppearance[] {
  return THEME_APPEARANCES.filter(
    (appearance) => themeColorsFor(theme, appearance) !== null
  )
}

/** The one appearance a theme has, or `null` when it has both. */
export function singleAppearanceOf(
  theme: ThemeDefinition
): ThemeAppearance | null {
  const modes = themeModes(theme)
  return modes.length === 1 ? modes[0]! : null
}

export const otherAppearance = (
  appearance: ThemeAppearance
): ThemeAppearance => (appearance === 'light' ? 'dark' : 'light')

export function themeIdFromName(name: string): string {
  const label = name.trim().toLowerCase()
  const normalized = label
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, THEME_NAME_MAX)
  if (normalized || !label) return normalized || 'custom-theme'
  let hash = 2_166_136_261
  for (let index = 0; index < label.length; index += 1) {
    hash ^= label.charCodeAt(index)
    hash = Math.imul(hash, 16_777_619)
  }
  return `theme-${(hash >>> 0).toString(36)}`
}

export type ThemeErrorCode =
  | 'not_an_object'
  | 'unsupported_version'
  | 'missing_name'
  | 'missing_appearance'
  | 'missing_colors'
  | 'invalid_id'
  | 'reserved_id'
  | 'unknown_role'
  | 'invalid_color'
  | 'no_colors'
  | 'invalid_collection'
  | 'invalid_variants'
  | 'variant_repeats_appearance'
  | 'already_installed'
  | 'not_installed'
  | 'empty_collection'
  | 'invalid_collection_themes'
  | 'palette_taken'
  | 'palettes_taken'

/** A theme that can't be read or saved; the UI words `code`, `subject` names what it's about. */
export class ThemeError extends Error {
  constructor(
    readonly code: ThemeErrorCode,
    readonly subject?: string,
    readonly appearance?: ThemeAppearance
  ) {
    super(subject ? `${code}: ${subject}` : code)
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

const isThemeAppearance = (value: unknown): value is ThemeAppearance =>
  value === 'light' || value === 'dark'

const isThemeId = (value: unknown): value is string =>
  typeof value === 'string' && /^[a-z0-9](?:[a-z0-9-]{0,47})$/.test(value)

const isThemeLabel = (value: unknown): value is string =>
  typeof value === 'string' &&
  value.trim().length > 0 &&
  value.trim().length <= THEME_NAME_MAX

function parseCollection(value: unknown): ThemeCollection | undefined {
  return isRecord(value) &&
    typeof value.id === 'string' &&
    /^[a-z0-9][a-z0-9.:-]{0,127}$/i.test(value.id) &&
    isThemeLabel(value.label)
    ? { id: value.id, label: value.label.trim() }
    : undefined
}

function lenientOverrides(
  value: Record<string, unknown>
): Partial<Record<ThemeColorRole, string>> {
  const overrides: Partial<Record<ThemeColorRole, string>> = {}
  for (const [role, color] of Object.entries(value)) {
    const normalized = canonicalColor(color)
    if (ROLE_SET.has(role) && normalized) {
      overrides[role as ThemeColorRole] = normalized
    }
  }
  return overrides
}

/** Reads one theme from storage, skipping unknown roles; `null` when it can't be used. */
export function parseStoredTheme(value: unknown): ThemeDefinition | null {
  if (!isRecord(value)) return null
  if (!isThemeId(value.id) || RESERVED_THEME_IDS.has(value.id)) return null
  if (!isThemeLabel(value.label) || !isThemeAppearance(value.appearance)) {
    return null
  }
  if (!isRecord(value.colors)) return null
  const colors = {
    ...standardColors(value.appearance),
    ...lenientOverrides(value.colors)
  }
  const variants: Partial<Record<ThemeAppearance, ThemeColors>> = {}
  if (value.variants !== undefined) {
    if (!isRecord(value.variants)) return null
    for (const [appearance, variant] of Object.entries(value.variants)) {
      if (!isThemeAppearance(appearance) || !isRecord(variant)) return null
      if (appearance === value.appearance) continue
      variants[appearance] = {
        ...standardColors(appearance),
        ...lenientOverrides(variant)
      }
    }
  }
  const collection = parseCollection(value.collection)
  return {
    id: value.id,
    label: value.label.trim(),
    appearance: value.appearance,
    colors,
    ...(Object.keys(variants).length > 0 ? { variants } : {}),
    ...(collection ? { collection } : {}),
    ...(value.managed === true ? { managed: true } : {})
  }
}

export function parseStoredThemes(value: unknown): ThemeDefinition[] {
  if (!Array.isArray(value)) return []
  const themes: ThemeDefinition[] = []
  for (const stored of value) {
    const theme = parseStoredTheme(stored)
    if (theme && !themes.some((existing) => existing.id === theme.id)) {
      themes.push(theme)
    }
  }
  return themes
}

function strictOverrides(
  value: unknown
): Partial<Record<ThemeColorRole, string>> {
  if (!isRecord(value)) throw new ThemeError('missing_colors')
  const overrides: Partial<Record<ThemeColorRole, string>> = {}
  for (const [role, color] of Object.entries(value)) {
    if (!ROLE_SET.has(role)) throw new ThemeError('unknown_role', role)
    const normalized = canonicalColor(color)
    if (!normalized) throw new ThemeError('invalid_color', role)
    overrides[role as ThemeColorRole] = normalized
  }
  if (Object.keys(overrides).length === 0) throw new ThemeError('no_colors')
  return overrides
}

/** Reads a hoshi theme file, `{ version: 1, name, appearance, colors, variants? }`. */
export function parseThemeFile(value: unknown): ThemeDefinition {
  if (!isRecord(value)) throw new ThemeError('not_an_object')
  if (value.version !== THEME_FILE_VERSION) {
    throw new ThemeError('unsupported_version')
  }
  const { name, appearance } = value
  if (!isThemeLabel(name)) throw new ThemeError('missing_name')
  if (!isThemeAppearance(appearance)) throw new ThemeError('missing_appearance')
  if (!isRecord(value.colors)) throw new ThemeError('missing_colors')
  const id = value.id === undefined ? themeIdFromName(name) : value.id
  if (!isThemeId(id)) throw new ThemeError('invalid_id')
  if (RESERVED_THEME_IDS.has(id)) throw new ThemeError('reserved_id', id)
  const collection = parseCollection(value.collection)
  if (value.collection !== undefined && !collection) {
    throw new ThemeError('invalid_collection')
  }
  const variants: Partial<Record<ThemeAppearance, ThemeColors>> = {}
  if (value.variants !== undefined) {
    if (!isRecord(value.variants)) throw new ThemeError('invalid_variants')
    for (const [variant, colors] of Object.entries(value.variants)) {
      if (!isThemeAppearance(variant)) throw new ThemeError('invalid_variants')
      if (variant === appearance) {
        throw new ThemeError('variant_repeats_appearance', appearance)
      }
      variants[variant] = {
        ...standardColors(variant),
        ...strictOverrides(colors)
      }
    }
  }
  return {
    id,
    label: name.trim(),
    appearance,
    colors: { ...standardColors(appearance), ...strictOverrides(value.colors) },
    ...(Object.keys(variants).length > 0 ? { variants } : {}),
    ...(collection ? { collection } : {}),
    ...(value.managed === true ? { managed: true } : {})
  }
}

export function serializeThemeFile(theme: ThemeDefinition): string {
  const file = {
    version: THEME_FILE_VERSION,
    id: theme.id,
    name: theme.label,
    appearance: theme.appearance,
    colors: theme.colors,
    ...(theme.variants ? { variants: theme.variants } : {}),
    ...(theme.collection ? { collection: theme.collection } : {}),
    ...(theme.managed ? { managed: true } : {})
  }
  return `${JSON.stringify(file, null, 2)}\n`
}

export function installTheme(
  themes: readonly ThemeDefinition[],
  theme: ThemeDefinition
): ThemeDefinition[] {
  if (RESERVED_THEME_IDS.has(theme.id)) {
    throw new ThemeError('reserved_id', theme.id)
  }
  if (themes.some((existing) => existing.id === theme.id)) {
    throw new ThemeError('already_installed', theme.label)
  }
  return [...themes, theme]
}

export function updateTheme(
  themes: readonly ThemeDefinition[],
  theme: ThemeDefinition
): ThemeDefinition[] {
  const index = themes.findIndex((existing) => existing.id === theme.id)
  if (index === -1) throw new ThemeError('not_installed', theme.label)
  const next = [...themes]
  next[index] = theme
  return next
}

export function removeThemes(
  themes: readonly ThemeDefinition[],
  ids: readonly string[]
): ThemeDefinition[] {
  const removed = new Set(ids)
  return themes.filter((theme) => !removed.has(theme.id))
}

/** Swaps an imported collection for its new themes, where the old ones stood. */
export function replaceCollection(
  themes: readonly ThemeDefinition[],
  collectionId: string,
  replacement: readonly ThemeDefinition[]
): ThemeDefinition[] {
  if (replacement.length === 0) throw new ThemeError('empty_collection')
  if (
    replacement.some((theme) => theme.collection?.id !== collectionId) ||
    new Set(replacement.map((theme) => theme.id)).size !== replacement.length
  ) {
    throw new ThemeError('invalid_collection_themes')
  }
  const others = themes.filter((theme) => theme.collection?.id !== collectionId)
  const taken = replacement.find(
    (theme) =>
      RESERVED_THEME_IDS.has(theme.id) ||
      others.some((other) => other.id === theme.id)
  )
  if (taken) throw new ThemeError('already_installed', taken.label)
  const at = themes.findIndex((theme) => theme.collection?.id === collectionId)
  if (at === -1) return [...others, ...replacement]
  const before = themes
    .slice(0, at)
    .filter((theme) => theme.collection?.id !== collectionId)
  const after = themes
    .slice(at)
    .filter((theme) => theme.collection?.id !== collectionId)
  return [...before, ...replacement, ...after]
}

/** Light and dark each picked from a different theme. A missing half follows the base theme. */
export type ThemeHalves = Readonly<{ light?: string; dark?: string }>

export type ThemeSelection = Readonly<{ theme: string; halves: ThemeHalves }>

export function parseHalves(value: unknown): ThemeHalves {
  if (!isRecord(value)) return {}
  const halves: { light?: string; dark?: string } = {}
  for (const appearance of THEME_APPEARANCES) {
    const id = value[appearance]
    if (typeof id === 'string') halves[appearance] = id
  }
  return halves
}

/** The halves that still name a theme with colors for their appearance. */
export function validHalves(
  halves: ThemeHalves,
  customThemes: readonly ThemeDefinition[]
): ThemeHalves {
  const valid: { light?: string; dark?: string } = {}
  for (const appearance of THEME_APPEARANCES) {
    const id = halves[appearance]
    if (id === undefined) continue
    const theme = findTheme(id, customThemes)
    if (theme && themeColorsFor(theme, appearance)) valid[appearance] = theme.id
  }
  return valid
}

/** Which card owns each half: `null` is the standard theme. */
export function halfOwners(
  selection: ThemeSelection,
  customThemes: readonly ThemeDefinition[]
): Record<ThemeAppearance, string | null> {
  const base = findTheme(selection.theme, customThemes)
  const baseId = base && base.id !== STANDARD_THEME_ID ? base.id : null
  const halves = validHalves(selection.halves, customThemes)
  const owner = (id: string | undefined) =>
    id === undefined ? baseId : id === STANDARD_THEME_ID ? null : id
  return { light: owner(halves.light), dark: owner(halves.dark) }
}

/** A whole theme replaces the mix. */
export function chooseTheme(id: string): ThemeSelection {
  return { theme: id, halves: {} }
}

/** Gives one half to a card; `null` gives it back to the standard theme. */
export function chooseHalf(
  selection: ThemeSelection,
  appearance: ThemeAppearance,
  cardId: string | null
): ThemeSelection {
  const other = otherAppearance(appearance)
  if (cardId === null && selection.theme !== STANDARD_THEME_ID) {
    const otherOwner = selection.halves[other] ?? selection.theme
    return { theme: STANDARD_THEME_ID, halves: { [other]: otherOwner } }
  }
  const halves: { light?: string; dark?: string } = { ...selection.halves }
  if (cardId === null) delete halves[appearance]
  else halves[appearance] = cardId
  return { theme: selection.theme, halves }
}

/** A card click: a theme with one appearance takes that half, any other the whole. */
export function chooseCard(
  selection: ThemeSelection,
  theme: ThemeDefinition
): ThemeSelection {
  const single = singleAppearanceOf(theme)
  return single === null
    ? chooseTheme(theme.id)
    : chooseHalf(selection, single, theme.id)
}

/** The selection once `ids` are gone: their halves fall back to the standard theme. */
export function selectionWithout(
  selection: ThemeSelection,
  ids: readonly string[]
): ThemeSelection {
  const removed = new Set(ids)
  const removesBase = removed.has(selection.theme)
  const halves: { light?: string; dark?: string } = removesBase
    ? {}
    : { ...selection.halves }
  for (const appearance of THEME_APPEARANCES) {
    const half = selection.halves[appearance]
    if (half === undefined) continue
    if (removed.has(half)) delete halves[appearance]
    else halves[appearance] = half
  }
  return { theme: removesBase ? STANDARD_THEME_ID : selection.theme, halves }
}

/**
 * The appearance the app shows: what the mode asks for, unless neither its half nor the base
 * theme has colors for it.
 */
export function resolveThemeAppearance(
  selection: ThemeSelection,
  mode: 'system' | ThemeAppearance,
  systemDark: boolean,
  customThemes: readonly ThemeDefinition[]
): ThemeAppearance {
  const wanted: ThemeAppearance =
    mode === 'system' ? (systemDark ? 'dark' : 'light') : mode
  if (validHalves(selection.halves, customThemes)[wanted]) return wanted
  const base = findTheme(selection.theme, customThemes)
  return base && themeColorsFor(base, wanted) === null
    ? base.appearance
    : wanted
}

/** The theme painting `appearance`, with its colors for it. */
export function resolveThemeFor(
  selection: ThemeSelection,
  appearance: ThemeAppearance,
  customThemes: readonly ThemeDefinition[]
): { theme: ThemeDefinition; colors: ThemeColors } {
  const halfId = validHalves(selection.halves, customThemes)[appearance]
  const theme =
    findTheme(halfId ?? selection.theme, customThemes) ?? BUILT_IN_THEMES[0]!
  const colors =
    themeColorsFor(theme, appearance) ??
    themeColorsFor(theme, theme.appearance)!
  return { theme, colors }
}

const DEFAULT_CANVAS: Record<ThemeAppearance, Rgb> = {
  light: { r: 250, g: 250, b: 250 },
  dark: { r: 24, g: 24, b: 27 }
}
const DEFAULT_ACCENT: Rgb = { r: 79, g: 70, b: 229 }

/** Every role from a background and an accent, the editor's simple mode. */
export function deriveThemeColors(
  appearance: ThemeAppearance,
  background: string,
  accentValue: string
): ThemeColors {
  const canvasRgb = colorToRgb(background, DEFAULT_CANVAS[appearance])
  const accentRgb = colorToRgb(accentValue, DEFAULT_ACCENT)
  const canvas = rgbToOklch(canvasRgb)
  const accent = rgbToOklch(accentRgb)
  const dark = isDarkRgb(canvasRgb)
  const hue = accent.c < 0.02 ? canvas.h : accent.h
  const tint = Math.min(0.045, Math.max(0.008, accent.c * 0.22))
  const step = dark ? 1 : -1
  const surfaceAt = (delta: number, chroma = tint): Oklch => ({
    l: Math.min(0.99, Math.max(0.05, canvas.l + step * delta)),
    c: chroma,
    h: hue
  })
  const color = (value: Oklch, alpha?: number) => oklchToColor(value, alpha)
  const textBase: Oklch = {
    l: dark ? 0.96 : 0.18,
    c: Math.min(0.03, accent.c * 0.2),
    h: hue
  }
  const text = solveLightness(
    textBase,
    [canvasRgb],
    7,
    dark ? 'lighter' : 'darker'
  )
  const textRgb = oklchToRgb(text)
  const foregroundOn = (surface: Oklch) =>
    color(
      solveLightness(
        textBase,
        [oklchToRgb(surface)],
        4.6,
        dark ? 'lighter' : 'darker'
      )
    )
  const card: Oklch = dark
    ? surfaceAt(0.06)
    : { l: Math.min(0.998, canvas.l + 0.007), c: tint * 0.6, h: hue }
  const secondary = surfaceAt(
    dark ? 0.12 : 0.03,
    Math.min(0.05, accent.c * 0.3)
  )
  const highlight = surfaceAt(
    dark ? 0.13 : 0.04,
    Math.min(0.06, accent.c * 0.4)
  )
  const sidebar = surfaceAt(dark ? 0.05 : 0.016, tint * 1.4)
  const sidebarAccent = surfaceAt(
    dark ? 0.13 : 0.05,
    Math.min(0.06, accent.c * 0.4)
  )
  const border = surfaceAt(dark ? 0.12 : 0.075, Math.min(0.04, accent.c * 0.25))
  const input = surfaceAt(dark ? 0.16 : 0.075, Math.min(0.04, accent.c * 0.25))
  const accentColor = rgbToColor(accentRgb)
  const accentForeground = rgbToColor(readableForeground(accentRgb))
  const ring = rgbToColor(mixRgb(accentRgb, canvasRgb, 0.35))
  const link = readableOn(accentColor, rgbToColor(canvasRgb), 4.5)
  const muted = rgbToColor(softenedText(canvasRgb, textRgb, dark ? 5 : 4.7))
  const standard = standardColors(appearance)
  return {
    ...standard,
    background: rgbToColor(canvasRgb),
    foreground: rgbToColor(textRgb),
    card: color(card),
    cardForeground: foregroundOn(card),
    popover: color(card),
    popoverForeground: foregroundOn(card),
    primary: accentColor,
    primaryForeground: accentForeground,
    secondary: color(secondary),
    secondaryForeground: foregroundOn(secondary),
    muted: color(secondary),
    mutedForeground: muted,
    accent: color(highlight),
    accentForeground: foregroundOn(highlight),
    border: color(border),
    input: color(input),
    ring,
    sidebar: color(sidebar),
    sidebarForeground: foregroundOn(sidebar),
    sidebarPrimary: accentColor,
    sidebarPrimaryForeground: accentForeground,
    sidebarAccent: color(sidebarAccent),
    sidebarAccentForeground: foregroundOn(sidebarAccent),
    sidebarBorder: color(border),
    sidebarRing: ring,
    header: color(surfaceAt(dark ? 0.045 : 0.025, tint * 1.2)),
    headerGlow: color(
      { l: 0.85, c: Math.min(0.06, accent.c), h: hue },
      dark ? 0.08 : 0.35
    ),
    link,
    send: link,
    terminalSelection: color(
      { l: dark ? 0.6 : 0.7, c: Math.min(0.08, accent.c), h: hue },
      dark ? 0.45 : 0.35
    ),
    scrollbar: rgbToColor(textRgb, dark ? 0.16 : 0.18),
    scrollbarHover: rgbToColor(textRgb, dark ? 0.3 : 0.34)
  }
}

/**
 * Sets one color in the editor's advanced mode, and the roles that pair with it: text that sits
 * on a surface follows it so it stays readable.
 */
export function updateColorFamily(
  colors: ThemeColors,
  role: ThemeColorRole,
  value: string
): ThemeColors {
  const parsed = parseColor(value)
  if (!parsed) return { ...colors, [role]: value }
  const normalized = oklchToColor(parsed.color, parsed.alpha)
  const canvas = colorToRgb(colors.background, WHITE)
  const selected = mixRgb(canvas, oklchToRgb(parsed.color), parsed.alpha)
  const foregroundOn = () => rgbToColor(readableForeground(selected))
  const textOn = () =>
    contrastRatio(colorToRgb(colors.foreground, BLACK), selected) >= 4.5
      ? colors.foreground
      : foregroundOn()
  switch (role) {
    case 'card':
    case 'popover':
      return {
        ...colors,
        card: normalized,
        popover: normalized,
        cardForeground: textOn(),
        popoverForeground: textOn()
      }
    case 'foreground':
      return {
        ...colors,
        foreground: normalized,
        cardForeground: normalized,
        popoverForeground: normalized
      }
    case 'border':
      return { ...colors, border: normalized, sidebarBorder: normalized }
    case 'secondary':
      return {
        ...colors,
        secondary: normalized,
        muted: normalized,
        secondaryForeground: textOn()
      }
    case 'accent':
      return { ...colors, accent: normalized, accentForeground: textOn() }
    case 'primary':
      return {
        ...colors,
        primary: normalized,
        primaryForeground: foregroundOn(),
        ring: rgbToColor(mixRgb(selected, canvas, 0.35)),
        sidebarPrimary: normalized,
        sidebarPrimaryForeground: foregroundOn(),
        sidebarRing: rgbToColor(mixRgb(selected, canvas, 0.35))
      }
    case 'link':
      return { ...colors, link: normalized, send: normalized }
    case 'sidebar':
      return { ...colors, sidebar: normalized, sidebarForeground: textOn() }
    case 'sidebarAccent':
      return {
        ...colors,
        sidebarAccent: normalized,
        sidebarAccentForeground: textOn()
      }
    case 'scrollbar':
      return {
        ...colors,
        scrollbar: normalized,
        scrollbarHover: oklchToColor(
          parsed.color,
          Math.min(1, parsed.alpha * 1.8)
        )
      }
    case 'destructive':
      return { ...colors, destructive: normalized, statusError: normalized }
    default:
      return { ...colors, [role]: normalized }
  }
}
