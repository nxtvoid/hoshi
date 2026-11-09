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

export const INTERFACE_FONTS = ['geist', 'inter', 'system'] as const
export type InterfaceFont = (typeof INTERFACE_FONTS)[number]

export const CODE_FONTS = ['jetbrains', 'cascadia', 'system'] as const
export type CodeFont = (typeof CODE_FONTS)[number]

/** The prompt box follows the interface font unless it is given one of the others. */
export const COMPOSER_FONTS = [
  'interface',
  ...INTERFACE_FONTS,
  'jetbrains',
  'cascadia',
  'system-mono'
] as const
export type ComposerFont = (typeof COMPOSER_FONTS)[number]

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
  interfaceFont: InterfaceFont
  interfaceSize: number
  codeFont: CodeFont
  codeSize: number
  /** Gives the prompt box and the terminal their own fonts instead of following the others. */
  separateFonts: boolean
  composerFont: ComposerFont
  composerSize: number
  terminalFont: CodeFont
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
    interfaceFont: oneOf(
      INTERFACE_FONTS,
      raw.interfaceFont,
      DEFAULT_APPEARANCE.interfaceFont
    ),
    interfaceSize: clampSize(raw.interfaceSize, INTERFACE_SIZE),
    codeFont: oneOf(CODE_FONTS, raw.codeFont, DEFAULT_APPEARANCE.codeFont),
    codeSize: clampSize(raw.codeSize, CODE_SIZE),
    separateFonts: flag(raw.separateFonts, DEFAULT_APPEARANCE.separateFonts),
    composerFont: oneOf(
      COMPOSER_FONTS,
      raw.composerFont,
      DEFAULT_APPEARANCE.composerFont
    ),
    composerSize: clampSize(raw.composerSize, COMPOSER_SIZE),
    terminalFont: oneOf(
      CODE_FONTS,
      raw.terminalFont,
      DEFAULT_APPEARANCE.terminalFont
    ),
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
  font: string | undefined
  codeFont: string | undefined
  composerFont: string | undefined
  terminalFont: string | undefined
  fontSmoothing: 'off' | undefined
  textWeight: 'regular' | undefined
  fontSize: string | undefined
  codeFontSize: string | undefined
  composerFontSize: string | undefined
  terminalFontSize: string | undefined
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
  return {
    dark: resolved === 'dark',
    palette: isBuiltInPalette(theme.id) ? theme.id : undefined,
    colors: builtIn ? undefined : colors,
    font:
      appearance.interfaceFont === DEFAULT_APPEARANCE.interfaceFont
        ? undefined
        : appearance.interfaceFont,
    codeFont:
      appearance.codeFont === DEFAULT_APPEARANCE.codeFont
        ? undefined
        : appearance.codeFont,
    composerFont:
      separate && appearance.composerFont !== 'interface'
        ? appearance.composerFont
        : undefined,
    terminalFont: separate ? appearance.terminalFont : undefined,
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
