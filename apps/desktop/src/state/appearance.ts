import {
  findTheme,
  isBuiltInPalette,
  parseHalves,
  parseStoredThemes,
  resolveThemeAppearance,
  resolveThemeFor,
  STANDARD_THEME_ID,
  type ThemeAppearance,
  type ThemeColors,
  type ThemeDefinition,
  type ThemeHalves,
  validHalves
} from './themes'

export const THEME_PREFERENCES = ['system', 'light', 'dark'] as const
export type ThemePreference = (typeof THEME_PREFERENCES)[number]

/** Fonts with a stack of their own in the stylesheet; any other value names an installed family. */
export const INTERFACE_FONTS = ['geist', 'inter', 'system'] as const
export const CODE_FONTS = ['jetbrains', 'cascadia', 'system'] as const
/** The prompt box follows the interface font unless it is given one of the others. */
export const COMPOSER_FONTS = [
  'interface',
  ...INTERFACE_FONTS,
  'jetbrains',
  'cascadia',
  'system-mono'
] as const

export type FontSlot = 'interface' | 'composer' | 'code' | 'terminal'

export const FONT_PRESETS: Record<FontSlot, readonly string[]> = {
  interface: INTERFACE_FONTS,
  composer: COMPOSER_FONTS,
  code: CODE_FONTS,
  terminal: CODE_FONTS
}

/** What an installed family falls back to for the glyphs it lacks. */
export const FONT_FALLBACKS: Record<FontSlot, string> = {
  interface: 'var(--stack-geist)',
  composer: 'var(--font-sans)',
  code: 'var(--stack-jetbrains)',
  terminal: 'var(--stack-jetbrains)'
}

const MAX_FAMILY_LENGTH = 200

export function isFontPreset(slot: FontSlot, value: string): boolean {
  return FONT_PRESETS[slot].includes(value)
}

/** A family name as a CSS string, safe to put in a `font-family` list. */
export function cssFontFamily(family: string): string {
  const name = family.replace(/[\n\r\f]/g, ' ').replace(/["\\]/g, '\\$&')
  return `"${name}"`
}

export function fontStack(slot: FontSlot, family: string): string {
  return `${cssFontFamily(family)}, ${FONT_FALLBACKS[slot]}`
}

export type SizeRange = { min: number; max: number; default: number }

export const INTERFACE_SIZE: SizeRange = { min: 12, max: 20, default: 16 }
export const COMPOSER_SIZE: SizeRange = { min: 12, max: 20, default: 14 }
export const CODE_SIZE: SizeRange = { min: 10, max: 18, default: 13 }
export const TERMINAL_SIZE: SizeRange = { min: 8, max: 20, default: 12 }

export function sizeOptions(range: SizeRange): number[] {
  return Array.from(
    { length: range.max - range.min + 1 },
    (_, index) => range.min + index
  )
}

export type Appearance = {
  preference: ThemePreference
  /** The whole theme: hoshi's own, a built-in palette or a custom theme. */
  theme: string
  halves: ThemeHalves
  customThemes: readonly ThemeDefinition[]
  /** A preset from `INTERFACE_FONTS` or the name of an installed family, as for the other fonts. */
  interfaceFont: string
  interfaceSize: number
  codeFont: string
  codeSize: number
  /** Gives the prompt box and the terminal their own fonts instead of following the others. */
  separateFonts: boolean
  composerFont: string
  composerSize: number
  terminalFont: string
  terminalSize: number
  /** Wraps long lines in code blocks and diffs. */
  wordWrap: boolean
  /** Thinner grayscale smoothing on macOS. */
  fontSmoothing: boolean
  lighterText: boolean
}

export const DEFAULT_APPEARANCE: Appearance = {
  preference: 'system',
  theme: STANDARD_THEME_ID,
  halves: {},
  customThemes: [],
  interfaceFont: 'geist',
  interfaceSize: INTERFACE_SIZE.default,
  codeFont: 'jetbrains',
  codeSize: CODE_SIZE.default,
  separateFonts: false,
  composerFont: 'interface',
  composerSize: COMPOSER_SIZE.default,
  terminalFont: 'jetbrains',
  terminalSize: TERMINAL_SIZE.default,
  wordWrap: true,
  fontSmoothing: true,
  lighterText: true
}

export function clampSize(value: unknown, range: SizeRange): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return range.default
  return Math.min(range.max, Math.max(range.min, Math.round(value)))
}

function fontChoice(value: unknown, fallback: string): string {
  if (typeof value !== 'string') return fallback
  const family = value.trim()
  return family.length > 0 && family.length <= MAX_FAMILY_LENGTH
    ? family
    : fallback
}

function oneOf<T extends string>(
  options: readonly T[],
  value: unknown,
  fallback: T
): T {
  return options.includes(value as T) ? (value as T) : fallback
}

/** Earlier versions saved a built-in palette as `palette`. */
function savedTheme(
  raw: Record<string, unknown>,
  customThemes: readonly ThemeDefinition[]
): string {
  for (const candidate of [raw.theme, raw.palette]) {
    if (typeof candidate !== 'string') continue
    const theme = findTheme(candidate, customThemes)
    if (theme) return theme.id
  }
  return STANDARD_THEME_ID
}

/** Reads what an earlier version saved; unknown or missing fields take their default. */
export function normalizeAppearance(saved: unknown): Appearance {
  const raw = (
    typeof saved === 'object' && saved !== null ? saved : {}
  ) as Record<string, unknown>
  const customThemes = parseStoredThemes(raw.customThemes)
  return {
    preference: oneOf(
      THEME_PREFERENCES,
      raw.preference,
      DEFAULT_APPEARANCE.preference
    ),
    theme: savedTheme(raw, customThemes),
    halves: validHalves(parseHalves(raw.halves), customThemes),
    customThemes,
    interfaceFont: fontChoice(
      raw.interfaceFont,
      DEFAULT_APPEARANCE.interfaceFont
    ),
    interfaceSize: clampSize(raw.interfaceSize, INTERFACE_SIZE),
    codeFont: fontChoice(raw.codeFont, DEFAULT_APPEARANCE.codeFont),
    codeSize: clampSize(raw.codeSize, CODE_SIZE),
    separateFonts: flag(raw.separateFonts, DEFAULT_APPEARANCE.separateFonts),
    composerFont: fontChoice(raw.composerFont, DEFAULT_APPEARANCE.composerFont),
    composerSize: clampSize(raw.composerSize, COMPOSER_SIZE),
    terminalFont: fontChoice(raw.terminalFont, DEFAULT_APPEARANCE.terminalFont),
    terminalSize: clampSize(raw.terminalSize, TERMINAL_SIZE),
    wordWrap: flag(raw.wordWrap, DEFAULT_APPEARANCE.wordWrap),
    fontSmoothing: flag(raw.fontSmoothing, DEFAULT_APPEARANCE.fontSmoothing),
    lighterText: flag(raw.lighterText, DEFAULT_APPEARANCE.lighterText)
  }
}

const flag = (value: unknown, fallback: boolean) =>
  typeof value === 'boolean' ? value : fallback

export function resolveAppearance(
  appearance: Appearance,
  systemDark: boolean
): ThemeAppearance {
  return resolveThemeAppearance(
    appearance,
    appearance.preference,
    systemDark,
    appearance.customThemes
  )
}

/**
 * What the root element carries for an appearance. `undefined` leaves the stylesheet's
 * default in place. `index.html` applies the same before the first paint.
 */
export type RootAppearance = {
  dark: boolean
  /** A built-in palette the stylesheet paints. */
  palette: string | undefined
  /** A custom theme's colors, set inline. */
  colors: ThemeColors | undefined
  /** A preset the stylesheet maps to its stack. */
  font: string | undefined
  codeFont: string | undefined
  composerFont: string | undefined
  terminalFont: string | undefined
  /** An installed family's stack, set inline as the font variable. */
  fontStack: string | undefined
  codeFontStack: string | undefined
  composerFontStack: string | undefined
  terminalFontStack: string | undefined
  fontSmoothing: 'off' | undefined
  textWeight: 'regular' | undefined
  fontSize: string | undefined
  codeFontSize: string | undefined
  composerFontSize: string | undefined
  terminalFontSize: string | undefined
}

type RootFont = { preset?: string; stack?: string }

function rootFont(slot: FontSlot, value: string, unchanged?: string): RootFont {
  if (value === unchanged) return {}
  if (isFontPreset(slot, value)) return { preset: value }
  return { stack: fontStack(slot, value) }
}

export function rootAppearance(
  appearance: Appearance,
  systemDark: boolean
): RootAppearance {
  const resolved = resolveAppearance(appearance, systemDark)
  const { theme, colors } = resolveThemeFor(
    appearance,
    resolved,
    appearance.customThemes
  )
  const interfaceSize = clampSize(appearance.interfaceSize, INTERFACE_SIZE)
  const codeSize = clampSize(appearance.codeSize, CODE_SIZE)
  const separate = appearance.separateFonts
  const builtIn = theme.id === STANDARD_THEME_ID || isBuiltInPalette(theme.id)
  const interfaceFont = rootFont(
    'interface',
    appearance.interfaceFont,
    DEFAULT_APPEARANCE.interfaceFont
  )
  const codeFont = rootFont(
    'code',
    appearance.codeFont,
    DEFAULT_APPEARANCE.codeFont
  )
  const composerFont = separate
    ? rootFont('composer', appearance.composerFont, 'interface')
    : {}
  const terminalFont = separate
    ? rootFont('terminal', appearance.terminalFont)
    : {}
  return {
    dark: resolved === 'dark',
    palette: isBuiltInPalette(theme.id) ? theme.id : undefined,
    colors: builtIn ? undefined : colors,
    font: interfaceFont.preset,
    codeFont: codeFont.preset,
    composerFont: composerFont.preset,
    terminalFont: terminalFont.preset,
    fontStack: interfaceFont.stack,
    codeFontStack: codeFont.stack,
    composerFontStack: composerFont.stack,
    terminalFontStack: terminalFont.stack,
    fontSmoothing: appearance.fontSmoothing ? undefined : 'off',
    textWeight: appearance.lighterText ? undefined : 'regular',
    fontSize:
      interfaceSize === INTERFACE_SIZE.default
        ? undefined
        : `${interfaceSize}px`,
    codeFontSize: codeSize === CODE_SIZE.default ? undefined : `${codeSize}px`,
    composerFontSize: separate
      ? `${clampSize(appearance.composerSize, COMPOSER_SIZE)}px`
      : undefined,
    terminalFontSize: separate
      ? `${clampSize(appearance.terminalSize, TERMINAL_SIZE)}px`
      : undefined
  }
}
