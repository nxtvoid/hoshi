export type Rgb = { r: number; g: number; b: number }
export type Oklch = { l: number; c: number; h: number }
export type ParsedColor = { color: Oklch; alpha: number }

export const WHITE: Rgb = { r: 255, g: 255, b: 255 }
export const BLACK: Rgb = { r: 0, g: 0, b: 0 }

/** At this luminance black and white text have the same contrast. */
const DARK_LUMINANCE = 0.179

const clamp = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value))

function srgbToLinear(channel: number): number {
  const c = channel / 255
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4
}

function linearToSrgb(channel: number, round: boolean): number {
  const c =
    channel <= 0.0031308
      ? channel * 12.92
      : 1.055 * channel ** (1 / 2.4) - 0.055
  const srgb = clamp(c, 0, 1) * 255
  return round ? Math.round(srgb) : srgb
}

export function rgbToOklch(color: Rgb): Oklch {
  const r = srgbToLinear(color.r)
  const g = srgbToLinear(color.g)
  const b = srgbToLinear(color.b)
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b)
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b)
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b)
  const lightness = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s
  const a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s
  const bb = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s
  const hue = (Math.atan2(bb, a) * 180) / Math.PI
  return { l: lightness, c: Math.hypot(a, bb), h: (hue + 360) % 360 }
}

function oklchToLinear({ l, c, h }: Oklch): Rgb {
  const radians = (h * Math.PI) / 180
  const a = c * Math.cos(radians)
  const b = c * Math.sin(radians)
  const lms = [
    (l + 0.3963377774 * a + 0.2158037573 * b) ** 3,
    (l - 0.1055613458 * a - 0.0638541728 * b) ** 3,
    (l - 0.0894841775 * a - 1.291485548 * b) ** 3
  ] as const
  return {
    r: 4.0767416621 * lms[0] - 3.3077115913 * lms[1] + 0.2309699292 * lms[2],
    g: -1.2684380046 * lms[0] + 2.6097574011 * lms[1] - 0.3413193965 * lms[2],
    b: -0.0041960863 * lms[0] - 0.7034186147 * lms[1] + 1.707614701 * lms[2]
  }
}

function inGamut(color: Oklch): boolean {
  const linear = oklchToLinear(color)
  return [linear.r, linear.g, linear.b].every(
    (channel) => channel >= -0.0001 && channel <= 1.0001
  )
}

/** Lowers the chroma until the color fits sRGB, keeping its lightness and hue. */
export function toSrgbGamut(color: Oklch): Oklch {
  if (inGamut(color)) return color
  let low = 0
  let high = color.c
  for (let step = 0; step < 24; step += 1) {
    const mid = (low + high) / 2
    if (inGamut({ ...color, c: mid })) low = mid
    else high = mid
  }
  return { ...color, c: low }
}

export function oklchToRgb(color: Oklch, round = true): Rgb {
  const linear = oklchToLinear(toSrgbGamut(color))
  return {
    r: linearToSrgb(linear.r, round),
    g: linearToSrgb(linear.g, round),
    b: linearToSrgb(linear.b, round)
  }
}

function formatNumber(value: number, precision: number): string {
  const rounded = Math.abs(value) < 10 ** -precision / 2 ? 0 : value
  return rounded.toFixed(precision).replace(/\.?0+$/, '')
}

export function formatOklch(color: Oklch, alpha = 1): string {
  const hue = color.c < 0.0005 ? 0 : ((color.h % 360) + 360) % 360
  const body = `${formatNumber(color.l, 4)} ${formatNumber(color.c, 4)} ${formatNumber(hue, 3)}`
  return alpha < 1
    ? `oklch(${body} / ${formatNumber(alpha, 3)})`
    : `oklch(${body})`
}

function parseHex(value: string): { rgb: Rgb; alpha: number } | null {
  const hex = value.slice(1)
  if (!/^(?:[0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(hex)) return null
  const short = hex.length <= 4
  const channel = (index: number) =>
    short
      ? Number.parseInt(hex[index]!.repeat(2), 16)
      : Number.parseInt(hex.slice(index * 2, index * 2 + 2), 16)
  const hasAlpha = hex.length === 4 || hex.length === 8
  return {
    rgb: { r: channel(0), g: channel(1), b: channel(2) },
    alpha: hasAlpha ? channel(3) / 255 : 1
  }
}

function parseNumber(part: string, percentScale: number): number | null {
  if (part === 'none') return 0
  const value = Number.parseFloat(part)
  if (!Number.isFinite(value) || !/^-?[\d.]+(?:e-?\d+)?%?$/i.test(part)) {
    return null
  }
  return part.endsWith('%') ? (value / 100) * percentScale : value
}

function splitArguments(body: string): { channels: string[]; alpha?: string } {
  const [channelPart = '', alphaPart] = body.split('/')
  const channels = channelPart
    .trim()
    .split(/[\s,]+/)
    .filter(Boolean)
  if (alphaPart !== undefined) {
    return { channels, alpha: alphaPart.trim() }
  }
  if (channels.length === 4) {
    return { channels: channels.slice(0, 3), alpha: channels[3] }
  }
  return { channels }
}

function parseAlpha(part: string | undefined): number | null {
  if (part === undefined) return 1
  const alpha = parseNumber(part, 1)
  return alpha === null ? null : clamp(alpha, 0, 1)
}

function hslToRgb(h: number, s: number, l: number): Rgb {
  const k = (n: number) => (n + h / 30) % 12
  const a = s * Math.min(l, 1 - l)
  const f = (n: number) =>
    l - a * Math.max(-1, Math.min(k(n) - 3, Math.min(9 - k(n), 1)))
  return { r: f(0) * 255, g: f(8) * 255, b: f(4) * 255 }
}

/** Reads hex, `rgb()`, `hsl()` and `oklch()` colors; anything else is `null`. */
export function parseColor(value: unknown): ParsedColor | null {
  if (typeof value !== 'string') return null
  const input = value.trim().toLowerCase()
  if (input.startsWith('#')) {
    const hex = parseHex(input)
    return hex ? { color: rgbToOklch(hex.rgb), alpha: hex.alpha } : null
  }
  const match = /^(rgba?|hsla?|oklch)\(([^()]*)\)$/.exec(input)
  if (!match) return null
  const [, name, body = ''] = match
  const { channels, alpha: alphaPart } = splitArguments(body)
  if (channels.length !== 3) return null
  const alpha = parseAlpha(alphaPart)
  if (alpha === null) return null
  if (name === 'oklch') {
    const l = parseNumber(channels[0]!, 1)
    const c = parseNumber(channels[1]!, 0.4)
    const h = parseNumber(channels[2]!.replace(/deg$/, ''), 360)
    if (l === null || c === null || h === null) return null
    return {
      color: {
        l: clamp(l, 0, 1),
        c: Math.max(0, c),
        h: ((h % 360) + 360) % 360
      },
      alpha
    }
  }
  if (name?.startsWith('rgb')) {
    const [r, g, b] = channels.map((part) => parseNumber(part, 255))
    if (r == null || g == null || b == null) return null
    return {
      color: rgbToOklch({
        r: clamp(r, 0, 255),
        g: clamp(g, 0, 255),
        b: clamp(b, 0, 255)
      }),
      alpha
    }
  }
  const h = parseNumber(channels[0]!.replace(/deg$/, ''), 360)
  const s = parseNumber(channels[1]!, 1)
  const l = parseNumber(channels[2]!, 1)
  if (h === null || s === null || l === null) return null
  return {
    color: rgbToOklch(
      hslToRgb(((h % 360) + 360) % 360, clamp(s, 0, 1), clamp(l, 0, 1))
    ),
    alpha
  }
}

export function isColor(value: unknown): value is string {
  return parseColor(value) !== null
}

/** The color as `oklch()`, the form themes store; `null` when it isn't a color. */
export function canonicalColor(value: unknown): string | null {
  const parsed = parseColor(value)
  return parsed ? formatOklch(parsed.color, parsed.alpha) : null
}

export function rgbToHex(color: Rgb): string {
  return `#${[color.r, color.g, color.b]
    .map((channel) =>
      Math.round(clamp(channel, 0, 255))
        .toString(16)
        .padStart(2, '0')
    )
    .join('')}`
}

export function colorToHex(value: string): string | null {
  const parsed = parseColor(value)
  if (!parsed) return null
  const hex = rgbToHex(oklchToRgb(parsed.color))
  if (parsed.alpha >= 1) return hex
  return `${hex}${Math.round(parsed.alpha * 255)
    .toString(16)
    .padStart(2, '0')}`
}

export function colorToRgb(value: string, fallback: Rgb): Rgb {
  const parsed = parseColor(value)
  return parsed ? oklchToRgb(parsed.color) : fallback
}

export function relativeLuminance(color: Rgb): number {
  return (
    0.2126 * srgbToLinear(color.r) +
    0.7152 * srgbToLinear(color.g) +
    0.0722 * srgbToLinear(color.b)
  )
}

export function isDarkRgb(color: Rgb): boolean {
  return relativeLuminance(color) < DARK_LUMINANCE
}

export function contrastRatio(first: Rgb, second: Rgb): number {
  const a = relativeLuminance(first)
  const b = relativeLuminance(second)
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05)
}

export function mixRgb(base: Rgb, overlay: Rgb, amount: number): Rgb {
  return {
    r: base.r + (overlay.r - base.r) * amount,
    g: base.g + (overlay.g - base.g) * amount,
    b: base.b + (overlay.b - base.b) * amount
  }
}

export function rgbToColor(color: Rgb, alpha = 1): string {
  return formatOklch(rgbToOklch(color), alpha)
}

export function oklchToColor(color: Oklch, alpha = 1): string {
  return formatOklch(toSrgbGamut(color), alpha)
}

/** Moves only the lightness until `base` reaches `minContrast` on every surface. */
export function solveLightness(
  base: Oklch,
  against: readonly Rgb[],
  minContrast: number,
  direction: 'lighter' | 'darker'
): Oklch {
  const contrastOf = (color: Oklch) =>
    Math.min(
      ...against.map((surface) => contrastRatio(oklchToRgb(color), surface))
    )
  if (contrastOf(base) >= minContrast) return base
  let low = direction === 'lighter' ? base.l : 0
  let high = direction === 'lighter' ? 1 : base.l
  for (let step = 0; step < 18; step += 1) {
    const mid = (low + high) / 2
    const passes = contrastOf({ ...base, l: mid }) >= minContrast
    if (direction === 'lighter') {
      if (passes) high = mid
      else low = mid
    } else if (passes) low = mid
    else high = mid
  }
  return { ...base, l: direction === 'lighter' ? high : low }
}

/** White or black, whichever reads better on `background`. */
export function readableForeground(background: Rgb): Rgb {
  return contrastRatio(background, WHITE) >= contrastRatio(background, BLACK)
    ? WHITE
    : BLACK
}

/** `foreground` softened toward `background` as far as `minimumRatio` allows. */
export function softenedText(
  background: Rgb,
  foreground: Rgb,
  minimumRatio: number
): Rgb {
  let readable = foreground
  let low = 0
  let high = 1
  for (let index = 0; index < 12; index += 1) {
    const amount = (low + high) / 2
    const candidate = mixRgb(foreground, background, amount)
    if (contrastRatio(candidate, background) >= minimumRatio) {
      readable = candidate
      low = amount
    } else {
      high = amount
    }
  }
  return readable
}

/** `color` with its lightness moved until it reads on `surface`, keeping its hue. */
export function readableOn(
  color: string,
  surface: string,
  minContrast: number
): string {
  const parsedColor = parseColor(color)
  const parsedSurface = parseColor(surface)
  if (!parsedColor || !parsedSurface) return color
  const surfaceRgb = oklchToRgb(parsedSurface.color)
  if (contrastRatio(oklchToRgb(parsedColor.color), surfaceRgb) >= minContrast) {
    return color
  }
  const solved = solveLightness(
    parsedColor.color,
    [surfaceRgb],
    minContrast,
    isDarkRgb(surfaceRgb) ? 'lighter' : 'darker'
  )
  return rgbToColor(oklchToRgb(solved))
}

export type Hsv = { h: number; s: number; v: number }

export function rgbToHsv({ r, g, b }: Rgb): Hsv {
  const [red, green, blue] = [r / 255, g / 255, b / 255]
  const max = Math.max(red, green, blue)
  const delta = max - Math.min(red, green, blue)
  let h = 0
  if (delta > 0) {
    if (max === red) h = ((green - blue) / delta) % 6
    else if (max === green) h = (blue - red) / delta + 2
    else h = (red - green) / delta + 4
  }
  return {
    h: (h * 60 + 360) % 360,
    s: max === 0 ? 0 : delta / max,
    v: max
  }
}

export function hsvToRgb({ h, s, v }: Hsv): Rgb {
  const f = (n: number) => {
    const k = (n + h / 60) % 6
    return (v - v * s * Math.max(0, Math.min(k, 4 - k, 1))) * 255
  }
  return { r: f(5), g: f(3), b: f(1) }
}
