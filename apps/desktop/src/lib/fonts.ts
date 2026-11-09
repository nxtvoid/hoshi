import type { SystemFont } from '@/bindings/SystemFont'

export type FontOption = { value: string; label: string }

/** Shipped with the app and offered as presets, so the installed copies would repeat them. */
const BUNDLED_FAMILIES = ['geist', 'geist variable', 'inter', 'inter variable']

const GENERIC_FAMILIES =
  /^(system-ui|ui-sans-serif|ui-serif|ui-monospace|sans-serif|serif|monospace|-apple-system|BlinkMacSystemFont)$/i

/** The installed families a picker offers, in the order they came. */
export function installedFamilies(
  fonts: readonly SystemFont[],
  monospace: boolean
): string[] {
  return fonts
    .filter(
      (font) =>
        (!monospace || font.monospace) &&
        !BUNDLED_FAMILIES.includes(font.family.toLowerCase())
    )
    .map((font) => font.family)
}

/** The presets, then the installed families, keeping those whose name contains the query. */
export function fontPickerValues(
  presets: readonly FontOption[],
  families: readonly string[],
  query: string
): string[] {
  const needle = query.trim().toLowerCase()
  const matches = (label: string) => label.toLowerCase().includes(needle)
  const presetValues = new Set(presets.map((preset) => preset.value))
  return [
    ...presets.filter((preset) => matches(preset.label)).map((p) => p.value),
    ...families.filter((family) => !presetValues.has(family) && matches(family))
  ]
}

/** The first named family of a CSS stack that is available, skipping generic families. */
export function firstAvailableFamily(
  stack: string,
  isAvailable: (family: string) => boolean
): string | null {
  for (const entry of stack.split(',')) {
    const family = entry.trim().replace(/^(['"])(.*)\1$/, '$2')
    if (family.length === 0 || GENERIC_FAMILIES.test(family)) continue
    if (isAvailable(family)) return family
  }
  return null
}
