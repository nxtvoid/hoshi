export const THEME_PREFERENCES = ['system', 'light', 'dark'] as const
export type ThemePreference = (typeof THEME_PREFERENCES)[number]

export const COLOR_PALETTES = [
  'hoshi',
  'grove',
  'ocean',
  'ember',
  'iris'
] as const
export type ColorPalette = (typeof COLOR_PALETTES)[number]

export const INTERFACE_FONTS = ['geist', 'inter', 'system'] as const
export type InterfaceFont = (typeof INTERFACE_FONTS)[number]

export const CODE_FONTS = ['jetbrains', 'cascadia', 'system'] as const
export type CodeFont = (typeof CODE_FONTS)[number]

export type SizeRange = { min: number; max: number; default: number }

export const INTERFACE_SIZE: SizeRange = { min: 12, max: 18, default: 16 }
export const CODE_SIZE: SizeRange = { min: 10, max: 18, default: 13 }

export type Appearance = {
  preference: ThemePreference
  palette: ColorPalette
  interfaceFont: InterfaceFont
  interfaceSize: number
  codeFont: CodeFont
  codeSize: number
  lighterText: boolean
}

export const DEFAULT_APPEARANCE: Appearance = {
  preference: 'system',
  palette: 'hoshi',
  interfaceFont: 'geist',
  interfaceSize: INTERFACE_SIZE.default,
  codeFont: 'jetbrains',
  codeSize: CODE_SIZE.default,
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

/** Reads what an earlier version saved; unknown or missing fields take their default. */
export function normalizeAppearance(saved: unknown): Appearance {
  const raw = (
    typeof saved === 'object' && saved !== null ? saved : {}
  ) as Record<string, unknown>
  return {
    preference: oneOf(
      THEME_PREFERENCES,
      raw.preference,
      DEFAULT_APPEARANCE.preference
    ),
    palette: oneOf(COLOR_PALETTES, raw.palette, DEFAULT_APPEARANCE.palette),
    interfaceFont: oneOf(
      INTERFACE_FONTS,
      raw.interfaceFont,
      DEFAULT_APPEARANCE.interfaceFont
    ),
    interfaceSize: clampSize(raw.interfaceSize, INTERFACE_SIZE),
    codeFont: oneOf(CODE_FONTS, raw.codeFont, DEFAULT_APPEARANCE.codeFont),
    codeSize: clampSize(raw.codeSize, CODE_SIZE),
    lighterText:
      typeof raw.lighterText === 'boolean'
        ? raw.lighterText
        : DEFAULT_APPEARANCE.lighterText
  }
}

export function resolveDark(
  preference: ThemePreference,
  systemDark: boolean
): boolean {
  return preference === 'dark' || (preference === 'system' && systemDark)
}

/**
 * What the root element carries for an appearance. `undefined` leaves the stylesheet's
 * default in place. `index.html` applies the same before the first paint.
 */
export type RootAppearance = {
  palette: string | undefined
  font: string | undefined
  codeFont: string | undefined
  textWeight: 'regular' | undefined
  fontSize: string | undefined
  codeFontSize: string | undefined
}

export function rootAppearance(appearance: Appearance): RootAppearance {
  const interfaceSize = clampSize(appearance.interfaceSize, INTERFACE_SIZE)
  const codeSize = clampSize(appearance.codeSize, CODE_SIZE)
  return {
    palette:
      appearance.palette === DEFAULT_APPEARANCE.palette
        ? undefined
        : appearance.palette,
    font:
      appearance.interfaceFont === DEFAULT_APPEARANCE.interfaceFont
        ? undefined
        : appearance.interfaceFont,
    codeFont:
      appearance.codeFont === DEFAULT_APPEARANCE.codeFont
        ? undefined
        : appearance.codeFont,
    textWeight: appearance.lighterText ? undefined : 'regular',
    fontSize:
      interfaceSize === INTERFACE_SIZE.default
        ? undefined
        : `${interfaceSize}px`,
    codeFontSize: codeSize === CODE_SIZE.default ? undefined : `${codeSize}px`
  }
}
