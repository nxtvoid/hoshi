/** A page an agent showed with `html_render`; mirrors `html_pages.rs` in hoshi-engine. */
export type HtmlPage = {
  id: string
  title: string
  height: number
  /** Its height at each width it was measured at, narrowest first. */
  heights: [number, number][] | null
}

export const HTML_RENDER_TOOL = 'mcp__hoshi__html_render'
export const PAGE_SCHEME = 'hoshi-page'

const COLUMN_WIDTH = 768
const MIN_HEIGHT = 80
const MAX_HEIGHT = 2000
/** The frame follows a live page this far: only a page that grows with its frame gets here. */
const MAX_FIT_HEIGHT = 20_000
const MAX_TITLE_CHARS = 200

const clampHeight = (height: number) =>
  Math.min(MAX_HEIGHT, Math.max(MIN_HEIGHT, Math.round(height)))

function readHeights(value: unknown): [number, number][] | null {
  if (!Array.isArray(value) || value.length === 0 || value.length > 24) {
    return null
  }
  const heights = value.flatMap((entry): [number, number][] =>
    Array.isArray(entry) &&
    entry.length === 2 &&
    Number.isInteger(entry[0]) &&
    entry[0] > 0 &&
    typeof entry[1] === 'number' &&
    Number.isFinite(entry[1])
      ? [[entry[0], clampHeight(entry[1])]]
      : []
  )
  return heights.length === value.length
    ? heights.sort((left, right) => left[0] - right[0])
    : null
}

/** The page in the output of an `html_render` call; `null` when it names none. */
export function readHtmlPage(output: string | null): HtmlPage | null {
  if (!output) return null
  let parsed: unknown
  try {
    parsed = JSON.parse(output)
  } catch {
    return null
  }
  const page = (parsed as { htmlRender?: unknown } | null)?.htmlRender
  if (typeof page !== 'object' || page === null) return null
  const { id, title, height, heights } = page as Record<string, unknown>
  if (
    typeof id !== 'string' ||
    !/^[a-z0-9]+_[a-z0-9]+$/i.test(id) ||
    typeof title !== 'string' ||
    typeof height !== 'number' ||
    !Number.isFinite(height)
  ) {
    return null
  }
  return {
    id,
    title: title.trim().slice(0, MAX_TITLE_CHARS),
    height: clampHeight(height),
    heights: readHeights(heights)
  }
}

/** The taller of the two measurements around `width`, as either may apply there. */
function measuredAt(heights: [number, number][], width: number) {
  const above = heights.findIndex(([measured]) => measured >= width)
  const high = above === -1 ? heights.length - 1 : above
  const low =
    (heights[high] as [number, number])[0] === width
      ? high
      : Math.max(0, high - 1)
  return Math.max(
    (heights[low] as [number, number])[1],
    (heights[high] as [number, number])[1]
  )
}

/**
 * How tall the frame is at `width`: the live page's height once it reported one, else what
 * was measured, capped at the height the agent asked for when it asked for less.
 */
export function frameHeight(
  page: HtmlPage,
  width: number,
  contentHeight: number | null
) {
  if (contentHeight !== null) {
    return Math.min(
      MAX_FIT_HEIGHT,
      Math.max(MIN_HEIGHT, Math.ceil(contentHeight))
    )
  }
  if (!page.heights) return page.height
  const cap =
    measuredAt(page.heights, COLUMN_WIDTH) > page.height
      ? page.height
      : MAX_HEIGHT
  return clampHeight(Math.min(cap, measuredAt(page.heights, width)))
}

type PageMessage =
  | { kind: 'height'; height: number }
  | { kind: 'link'; url: string }

/** What a page's frame posts: its height, or a link to open. */
export function readPageMessage(data: unknown): PageMessage | null {
  if (typeof data !== 'object' || data === null) return null
  const { jsonrpc, method, params } = data as Record<string, unknown>
  if (jsonrpc !== '2.0' || typeof params !== 'object' || params === null) {
    return null
  }
  const { height, url } = params as Record<string, unknown>
  if (
    method === 'ui/notifications/size-changed' &&
    typeof height === 'number' &&
    Number.isFinite(height) &&
    height > 0
  ) {
    return { kind: 'height', height }
  }
  if (
    method === 'ui/open-link' &&
    typeof url === 'string' &&
    /^https?:\/\//i.test(url)
  ) {
    return { kind: 'link', url }
  }
  return null
}

export type PageTheme = {
  appearance: 'light' | 'dark'
  variables: Record<string, string>
}

/** Each variable of a page's theme, and the app's token it takes its value from. */
export const PAGE_THEME_TOKENS: Record<string, string> = {
  '--background': '--background',
  '--foreground': '--foreground',
  '--muted': '--muted',
  '--muted-foreground': '--muted-foreground',
  '--card': '--card',
  '--card-foreground': '--card-foreground',
  '--popover': '--popover',
  '--popover-foreground': '--popover-foreground',
  '--secondary': '--secondary',
  '--secondary-foreground': '--secondary-foreground',
  '--border': '--border',
  '--input': '--input',
  '--ring': '--ring',
  '--primary': '--primary',
  '--primary-foreground': '--primary-foreground',
  '--accent': '--send',
  '--accent-foreground': '--background',
  '--destructive': '--status-error',
  '--destructive-foreground': '--background',
  '--warning': '--warning',
  '--warning-foreground': '--background',
  '--success': '--status-done',
  '--success-foreground': '--background',
  '--info': '--status-working',
  '--info-foreground': '--background',
  '--code-background': '--muted',
  '--code-foreground': '--foreground',
  '--chart-1': '--link',
  '--chart-2': '--status-done',
  '--chart-3': '--status-waiting',
  '--chart-4': '--pr-merged',
  '--chart-5': '--status-error',
  '--chart-6': '--file-teal',
  '--radius': '--radius',
  '--font-sans': '--font-sans',
  '--font-mono': '--font-mono'
}

/** Fonts the app bundles aren't in the page's frame, so its own stack ends in the system's. */
const FALLBACK_FONTS: Record<string, string> = {
  '--font-sans': 'system-ui, "Segoe UI", sans-serif',
  '--font-mono': 'Consolas, Menlo, monospace'
}

/** The page theme from the app's current values of its tokens. */
export function pageTheme(
  dark: boolean,
  token: (name: string) => string
): PageTheme {
  const variables: Record<string, string> = {}
  for (const [variable, source] of Object.entries(PAGE_THEME_TOKENS)) {
    const value = token(source).trim()
    if (!value) continue
    const fallback = FALLBACK_FONTS[variable]
    variables[variable] = fallback ? `${value}, ${fallback}` : value
  }
  return { appearance: dark ? 'dark' : 'light', variables }
}

/** The address fragment a page reads its first theme from, before it paints. */
export function themeFragment(theme: PageTheme) {
  return `#hoshi-theme=${encodeURIComponent(JSON.stringify(theme))}`
}

export function themeMessage(theme: PageTheme) {
  return {
    jsonrpc: '2.0',
    method: 'ui/notifications/host-context-changed',
    params: { theme: theme.appearance, styles: { variables: theme.variables } }
  }
}
