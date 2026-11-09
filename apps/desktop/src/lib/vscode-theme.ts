import type { ExtensionTheme } from '@/bindings/ExtensionTheme'
import type { ThemeExtension } from '@/bindings/ThemeExtension'
import {
  canonicalColor,
  colorToHex,
  colorToRgb,
  contrastRatio,
  isDarkRgb,
  mixRgb,
  type Rgb,
  readableForeground,
  readableOn,
  rgbToColor,
  rgbToHex,
  WHITE
} from './color'
import {
  deriveThemeColors,
  parseThemeFile,
  RESERVED_THEME_IDS,
  standardColors,
  THEME_FILE_VERSION,
  THEME_NAME_MAX,
  type ThemeAppearance,
  type ThemeColorRole,
  type ThemeDefinition,
  ThemeError,
  themeIdFromName,
  themeModes
} from '@/state/themes'

type Rgba = Rgb & { a: number }

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** Reads JSON with comments and trailing commas, as VS Code writes it; throws when it can't. */
export function parseJsonc(source: string): unknown {
  let stripped = ''
  let inString = false
  const text = source.replace(/^﻿/, '')
  for (let index = 0; index < text.length; index += 1) {
    const c = text[index]!
    if (inString) {
      stripped += c
      if (c === '\\') {
        stripped += text[index + 1] ?? ''
        index += 1
      } else if (c === '"') inString = false
      continue
    }
    if (c === '"') {
      inString = true
      stripped += c
    } else if (c === '/' && text[index + 1] === '/') {
      while (index < text.length && text[index] !== '\n') index += 1
      stripped += '\n'
    } else if (c === '/' && text[index + 1] === '*') {
      const end = text.indexOf('*/', index + 2)
      index = end === -1 ? text.length : end + 1
      stripped += ' '
    } else stripped += c
  }
  let result = ''
  inString = false
  for (let index = 0; index < stripped.length; index += 1) {
    const c = stripped[index]!
    if (inString) {
      result += c
      if (c === '\\') {
        result += stripped[index + 1] ?? ''
        index += 1
      } else if (c === '"') inString = false
    } else if (c === '"') {
      inString = true
      result += c
    } else if (c === ',') {
      const next = stripped.slice(index + 1).match(/\S/)?.[0]
      if (next !== '}' && next !== ']') result += c
    } else result += c
  }
  return JSON.parse(result)
}

function decodeGamma(value: number) {
  return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
}

function encodeGamma(value: number) {
  const clamped = Math.max(0, Math.min(1, value))
  return clamped <= 0.0031308
    ? clamped * 12.92
    : 1.055 * clamped ** (1 / 2.4) - 0.055
}

function parseColorFunction(value: string): Rgba | null {
  const match = /^color\(\s*(display-p3|srgb)\s+([^)]+)\)$/i.exec(value)
  if (!match) return null
  const [channelPart = '', alphaPart] = match[2]!.split('/')
  const channels = channelPart
    .trim()
    .split(/\s+/)
    .map((part) =>
      part.endsWith('%')
        ? Number.parseFloat(part) / 100
        : Number.parseFloat(part)
    )
  if (channels.length !== 3 || channels.some((c) => !Number.isFinite(c))) {
    return null
  }
  const alphaRaw = alphaPart?.trim()
  const alpha =
    alphaRaw === undefined
      ? 1
      : alphaRaw.endsWith('%')
        ? Number.parseFloat(alphaRaw) / 100
        : Number.parseFloat(alphaRaw)
  if (!Number.isFinite(alpha)) return null
  const a = Math.max(0, Math.min(1, alpha))
  const [red, green, blue] = channels as [number, number, number]
  if (match[1]!.toLowerCase() === 'srgb') {
    return { r: red * 255, g: green * 255, b: blue * 255, a }
  }
  const [lr, lg, lb] = [red, green, blue].map(decodeGamma) as [
    number,
    number,
    number
  ]
  return {
    r: encodeGamma(1.2249401762805 * lr - 0.2249401762805 * lg) * 255,
    g: encodeGamma(-0.042056961239 * lr + 1.042056961239 * lg) * 255,
    b:
      encodeGamma(
        -0.0196375547643 * lr - 0.0786360655012 * lg + 1.0982736202656 * lb
      ) * 255,
    a
  }
}

function parseVsCodeColor(value: unknown): Rgba | null {
  if (typeof value !== 'string') return null
  const trimmed = value.trim()
  if (trimmed.startsWith('color(')) return parseColorFunction(trimmed)
  const hex = trimmed.replace(/^#/, '')
  if (!/^(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(hex)) return null
  const expand = (part: string) =>
    Number.parseInt(part.length === 1 ? part + part : part, 16)
  if (hex.length <= 4) {
    return {
      r: expand(hex[0]!),
      g: expand(hex[1]!),
      b: expand(hex[2]!),
      a: hex.length === 4 ? expand(hex[3]!) / 255 : 1
    }
  }
  return {
    r: expand(hex.slice(0, 2)),
    g: expand(hex.slice(2, 4)),
    b: expand(hex.slice(4, 6)),
    a: hex.length === 8 ? expand(hex.slice(6, 8)) / 255 : 1
  }
}

/** A translucent VS Code overlay composited onto its surface; hoshi's surfaces are opaque. */
function flattenOver(color: Rgba, base: Rgb): string {
  return rgbToHex(color.a >= 1 ? color : mixRgb(base, color, color.a))
}

const hexRgb = (value: string) => colorToRgb(value, WHITE)

export function isVsCodeThemeFile(value: unknown): boolean {
  if (!isRecord(value) || value.version === THEME_FILE_VERSION) return false
  const workbench =
    isRecord(value.colors) &&
    Object.keys(value.colors).some((key) => key.includes('.'))
  return workbench || Array.isArray(value.tokenColors)
}

export function humanizeThemeName(raw: string): string {
  const trimmed = raw.trim()
  if (/\s/.test(trimmed) || !/[-_.]/.test(trimmed)) return trimmed
  return trimmed
    .split(/[-_.]+/)
    .filter(Boolean)
    .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
    .join(' ')
}

function resolveName(value: Record<string, unknown>, fallback: string): string {
  for (const candidate of [value.displayName, value.name]) {
    if (typeof candidate !== 'string') continue
    const humanized = humanizeThemeName(candidate)
    if (humanized) return humanized.slice(0, THEME_NAME_MAX)
  }
  return fallback
}

function resolveAppearance(
  value: Record<string, unknown>,
  canvas: Rgb
): ThemeAppearance {
  const type = typeof value.type === 'string' ? value.type.toLowerCase() : null
  if (type === 'light' || type === 'hc-light') return 'light'
  if (type === 'dark' || type === 'hc-black') return 'dark'
  return isDarkRgb(canvas) ? 'dark' : 'light'
}

/** A hoshi theme from a VS Code color theme's workbench colors. */
export function parseVsCodeThemeFile(
  value: unknown,
  fallbackName: string
): ThemeDefinition {
  if (!isRecord(value)) throw new ThemeError('not_an_object')
  const colors = isRecord(value.colors) ? value.colors : {}
  const pick = (...keys: string[]) => {
    for (const key of keys) {
      const parsed = parseVsCodeColor(colors[key])
      if (parsed) return parsed
    }
    return null
  }
  const solidOver = (base: Rgb, ...keys: string[]) => {
    const parsed = pick(...keys)
    return parsed ? flattenOver(parsed, base) : null
  }
  const canvasColor = pick('editor.background', 'editorPane.background')
  if (!canvasColor) throw new ThemeError('vscode_no_background')
  const canvas: Rgb = { r: canvasColor.r, g: canvasColor.g, b: canvasColor.b }
  const appearance = resolveAppearance(value, canvas)
  const canvasHex = rgbToHex(canvas)
  const raisedHex = solidOver(
    canvas,
    'editorWidget.background',
    'dropdown.background'
  )
  const standsApart = (first: string, second: string) =>
    contrastRatio(hexRgb(first), hexRgb(second)) >= 1.1

  let accentHex: string | null = null
  for (const key of [
    'focusBorder',
    'button.background',
    'textLink.foreground',
    'activityBarBadge.background',
    'progressBar.background',
    'badge.background'
  ]) {
    const candidate = parseVsCodeColor(colors[key])
    if (!candidate) continue
    const candidateHex = flattenOver(candidate, canvas)
    if (
      !standsApart(candidateHex, canvasHex) ||
      (raisedHex !== null && !standsApart(candidateHex, raisedHex))
    ) {
      continue
    }
    accentHex = candidateHex
    break
  }
  if (!accentHex) {
    const standard = colorToHex(standardColors(appearance).primary)!
    accentHex =
      [standard, '#ffffff', '#000000'].find(
        (candidate) =>
          standsApart(candidate, canvasHex) &&
          (raisedHex === null || standsApart(candidate, raisedHex))
      ) ?? standard
  }
  const accentRgb = hexRgb(accentHex)
  const derived = deriveThemeColors(
    appearance,
    canvasHex,
    rgbToHex(mixRgb(canvas, accentRgb, 0.2))
  )
  const readableOnSurface = (
    surface: string,
    fallback: string,
    ...keys: string[]
  ) => {
    const surfaceRgb = hexRgb(surface)
    const readable = (candidate: string) =>
      contrastRatio(hexRgb(candidate), surfaceRgb) >= 4.5
    const specified = solidOver(surfaceRgb, ...keys)
    if (specified && readable(specified)) return specified
    if (readable(fallback)) return fallback
    return rgbToColor(readableForeground(surfaceRgb))
  }
  const statusColor = (fallback: string, ...keys: string[]) => {
    const specified = solidOver(canvas, ...keys)
    const base =
      specified && standsApart(specified, canvasHex) ? specified : fallback
    return readableOn(base, canvasHex, 4.5)
  }
  const translucent = (fallback: string, ...keys: string[]) => {
    for (const key of keys) {
      const color = canonicalColor(colors[key])
      if (color) return color
    }
    return fallback
  }

  const cardHex = solidOver(canvas, 'editorWidget.background') ?? derived.card
  const popoverHex =
    solidOver(
      canvas,
      'menu.background',
      'quickInput.background',
      'dropdown.background'
    ) ?? cardHex
  const buttonHex = solidOver(canvas, 'button.background')
  const sendHex =
    buttonHex && standsApart(buttonHex, canvasHex) ? buttonHex : accentHex
  const inputCandidates = [
    derived.input,
    derived.card,
    standardColors(appearance).input,
    '#000000',
    '#ffffff',
    '#808080'
  ]
  let inputHex =
    inputCandidates.find((candidate) => standsApart(candidate, canvasHex)) ??
    '#808080'
  for (const key of ['input.background', 'input.border']) {
    const candidate = solidOver(canvas, key)
    if (candidate && standsApart(candidate, canvasHex)) {
      inputHex = candidate
      break
    }
  }
  const sidebarHex =
    solidOver(canvas, 'sideBar.background', 'activityBar.background') ??
    derived.sidebar
  const sidebar = hexRgb(sidebarHex)
  const highlightHex =
    solidOver(
      canvas,
      'list.activeSelectionBackground',
      'list.hoverBackground'
    ) ?? derived.accent
  const sidebarSelectionHex =
    solidOver(
      sidebar,
      'list.activeSelectionBackground',
      'list.inactiveSelectionBackground',
      'list.hoverBackground'
    ) ?? derived.sidebarAccent
  const foreground = readableOnSurface(
    canvasHex,
    derived.foreground,
    'editor.foreground',
    'foreground'
  )
  const ring = rgbToColor(mixRgb(accentRgb, canvas, 0.35))
  const primaryForeground = readableOnSurface(
    accentHex,
    derived.primaryForeground,
    'button.foreground'
  )
  const error = statusColor(
    derived.destructive,
    'editorError.foreground',
    'errorForeground'
  )

  const overrides: Partial<Record<ThemeColorRole, string>> = {
    background: canvasHex,
    foreground,
    card: cardHex,
    cardForeground: readableOnSurface(cardHex, foreground, 'editor.foreground'),
    popover: popoverHex,
    popoverForeground: readableOnSurface(
      popoverHex,
      foreground,
      'editor.foreground'
    ),
    primary: accentHex,
    primaryForeground,
    mutedForeground: readableOnSurface(
      canvasHex,
      derived.mutedForeground,
      'descriptionForeground',
      'disabledForeground'
    ),
    accent: highlightHex,
    accentForeground: readableOnSurface(highlightHex, foreground),
    destructive: error,
    statusError: error,
    statusWaiting: statusColor(
      derived.statusWaiting,
      'editorWarning.foreground'
    ),
    border:
      solidOver(
        canvas,
        'panel.border',
        'editorGroup.border',
        'contrastBorder'
      ) ?? derived.border,
    input: inputHex,
    ring,
    sidebar: sidebarHex,
    sidebarForeground: readableOnSurface(
      sidebarHex,
      derived.sidebarForeground,
      'sideBar.foreground'
    ),
    sidebarPrimary: accentHex,
    sidebarPrimaryForeground: primaryForeground,
    sidebarAccent: sidebarSelectionHex,
    sidebarAccentForeground: readableOnSurface(sidebarSelectionHex, foreground),
    sidebarBorder:
      solidOver(sidebar, 'sideBar.border') ?? derived.sidebarBorder,
    sidebarRing: ring,
    link: readableOn(
      solidOver(canvas, 'textLink.foreground') ?? accentHex,
      canvasHex,
      4.5
    ),
    send: sendHex,
    terminalSelection: translucent(
      derived.terminalSelection,
      'terminal.selectionBackground',
      'editor.selectionBackground'
    ),
    scrollbar: translucent(derived.scrollbar, 'scrollbarSlider.background')
  }

  const name = resolveName(value, fallbackName)
  const generatedId = themeIdFromName(name)
  return parseThemeFile({
    version: THEME_FILE_VERSION,
    ...(RESERVED_THEME_IDS.has(generatedId)
      ? { id: `${generatedId}-vscode` }
      : {}),
    name,
    appearance,
    colors: { ...derived, ...overrides }
  })
}

/** A hoshi theme file or a VS Code color theme, from its JSON text. */
export function parseThemeImport(
  source: string,
  fallbackName: string
): ThemeDefinition {
  let value: unknown
  try {
    value = parseJsonc(source)
  } catch {
    throw new ThemeError('invalid_json')
  }
  return isVsCodeThemeFile(value)
    ? parseVsCodeThemeFile(value, fallbackName)
    : parseThemeFile(value)
}

function rename(theme: ThemeDefinition, name: string): ThemeDefinition | null {
  try {
    return parseThemeFile({
      version: THEME_FILE_VERSION,
      name: name.slice(0, THEME_NAME_MAX),
      appearance: theme.appearance,
      colors: theme.colors,
      ...(theme.variants ? { variants: theme.variants } : {}),
      ...(theme.managed ? { managed: true } : {})
    })
  } catch {
    return null
  }
}

/** Joins a light and a dark theme that differ only by "Light" and "Dark" in their names. */
export function pairVsCodeThemes(
  themes: readonly ThemeDefinition[],
  pairedId?: (light: ThemeDefinition, dark: ThemeDefinition) => string
): ThemeDefinition[] {
  const stripAppearance = (label: string) =>
    label
      .replace(/\b(?:light|dark)\b/gi, ' ')
      .replace(/\s+/g, ' ')
      .trim()
  type Group = {
    light: ThemeDefinition[]
    dark: ThemeDefinition[]
    order: number
  }
  const groups = new Map<string, Group>()
  const entries: { theme: ThemeDefinition; order: number }[] = []
  themes.forEach((theme, order) => {
    const key = stripAppearance(theme.label)
    if (
      themeModes(theme).length !== 1 ||
      key === theme.label ||
      !/[a-z0-9]/i.test(key)
    ) {
      entries.push({ theme, order })
      return
    }
    const group = groups.get(key) ?? { light: [], dark: [], order }
    group[theme.appearance].push(theme)
    groups.set(key, group)
  })
  for (const [key, group] of groups) {
    const [light] = group.light
    const [dark] = group.dark
    if (group.light.length === 1 && group.dark.length === 1 && light && dark) {
      try {
        entries.push({
          order: group.order,
          theme: parseThemeFile({
            version: THEME_FILE_VERSION,
            ...(pairedId ? { id: pairedId(light, dark) } : {}),
            name: key,
            appearance: 'light',
            colors: light.colors,
            variants: { dark: dark.colors }
          })
        })
        continue
      } catch {}
    }
    for (const theme of [...group.light, ...group.dark]) {
      entries.push({ theme, order: group.order })
    }
  }
  return entries.sort((a, b) => a.order - b.order).map((entry) => entry.theme)
}

/** Themes that would share an id are renamed after their file, then numbered. */
export function resolveThemeLabelCollisions(
  entries: readonly { theme: ThemeDefinition; sourceName?: string }[]
): ThemeDefinition[] {
  const counts = new Map<string, number>()
  for (const { theme } of entries) {
    counts.set(theme.id, (counts.get(theme.id) ?? 0) + 1)
  }
  const relabelled = entries.map(({ theme, sourceName }) => {
    if ((counts.get(theme.id) ?? 0) < 2) return theme
    const stem = sourceName?.replace(/\.[^.]+$/, '')
    const fromFile = stem ? humanizeThemeName(stem) : null
    const renamed =
      fromFile && fromFile.toLowerCase() !== theme.label.toLowerCase()
        ? rename(theme, fromFile)
        : null
    return renamed ?? theme
  })
  const seen = new Set<string>()
  return relabelled.map((theme) => {
    if (!seen.has(theme.id)) {
      seen.add(theme.id)
      return theme
    }
    for (let suffix = 2; suffix < 100; suffix += 1) {
      const candidate = rename(theme, `${theme.label} ${suffix}`)
      if (candidate && !seen.has(candidate.id)) {
        seen.add(candidate.id)
        return candidate
      }
    }
    return theme
  })
}

function shortHash(text: string): string {
  let first = 0x811c9dc5
  let second = 0x01000193 ^ text.length
  for (let index = 0; index < text.length; index += 1) {
    const code = text.charCodeAt(index)
    first = Math.imul(first ^ code, 0x01000193)
    second = Math.imul(second ^ code, 0x5bd1e995)
  }
  return `${(first >>> 0).toString(16).padStart(8, '0')}${(second >>> 0)
    .toString(16)
    .padStart(8, '0')
    .slice(0, 4)}`
}

function openVsxThemeId(extensionId: string, source: string): string {
  return `ovx-theme-${shortHash(`${extensionId}:${source}`)}`
}

/** The themes an Open VSX extension installs: each file read, paired and grouped. */
export function extensionThemes(
  extension: ThemeExtension,
  files: readonly ExtensionTheme[]
): ThemeDefinition[] {
  if (files.length === 0) throw new ThemeError('no_compatible_themes')
  const extensionId = extension.id.toLowerCase()
  const parsed = files.map((file) => ({
    theme: parseVsCodeThemeFile(
      {
        colors: file.colors,
        displayName: file.label,
        ...(file.uiTheme ? { type: file.uiTheme } : {})
      },
      extension.name.slice(0, THEME_NAME_MAX)
    ),
    sourceName: file.sourcePath.split('/').at(-1)!,
    sourcePath: file.sourcePath
  }))
  const resolved = resolveThemeLabelCollisions(parsed).map((theme, index) => ({
    ...theme,
    id: openVsxThemeId(extensionId, parsed[index]!.sourcePath)
  }))
  const paired = pairVsCodeThemes(resolved, (light, dark) =>
    openVsxThemeId(extensionId, [light.id, dark.id].sort().join(':'))
  )
  const collection = {
    id: extension.collectionId,
    label: extension.name.slice(0, THEME_NAME_MAX)
  }
  const ids = paired.map((theme) => theme.id)
  return resolveThemeLabelCollisions(paired.map((theme) => ({ theme }))).map(
    (theme, index) => ({ ...theme, id: ids[index] ?? theme.id, collection })
  )
}
