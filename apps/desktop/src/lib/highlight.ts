import type { BundledLanguage, Highlighter } from 'shiki'

const THEMES = { light: 'github-light', dark: 'github-dark' } as const
const CACHE_LIMIT = 200

const cache = new Map<string, string>()
let highlighter: Promise<Highlighter> | null = null

function load() {
  highlighter ??= Promise.all([
    import('shiki'),
    import('shiki/engine/javascript')
  ]).then(([shiki, engine]) =>
    shiki.createHighlighter({
      themes: Object.values(THEMES),
      langs: [],
      engine: engine.createJavaScriptRegexEngine()
    })
  )
  return highlighter
}

const cacheKey = (code: string, language: string) => `${language}\u0000${code}`

export function cachedHighlight(code: string, language: string) {
  return cache.get(cacheKey(code, language))
}

/**
 * The code as HTML with both themes' colors in `--shiki-light` and `--shiki-dark`;
 * `null` for a language Shiki does not know.
 */
export async function highlight(
  code: string,
  language: string
): Promise<string | null> {
  const { bundledLanguages } = await import('shiki')
  if (!Object.hasOwn(bundledLanguages, language)) return null
  const shiki = await load()
  await shiki.loadLanguage(language as BundledLanguage)
  const html = shiki.codeToHtml(code, {
    lang: language,
    themes: THEMES,
    defaultColor: false
  })
  cache.set(cacheKey(code, language), html)
  if (cache.size > CACHE_LIMIT) {
    const oldest = cache.keys().next().value
    if (oldest !== undefined) cache.delete(oldest)
  }
  return html
}

/** A colored run of code: offsets into the code and its `--shiki-*` colors. */
export type CodeToken = { from: number; to: number; style: string }

/** The code's colored runs; `null` for a language Shiki does not know. */
export async function highlightTokens(
  code: string,
  language: string
): Promise<CodeToken[] | null> {
  const lang = language.trim().toLowerCase()
  const { bundledLanguages } = await import('shiki')
  if (!lang || !Object.hasOwn(bundledLanguages, lang)) return null
  const shiki = await load()
  await shiki.loadLanguage(lang as BundledLanguage)
  const { tokens } = shiki.codeToTokens(code, {
    lang: lang as BundledLanguage,
    themes: THEMES,
    defaultColor: false
  })
  return tokens.flat().flatMap((token) =>
    token.htmlStyle && token.content
      ? [
          {
            from: token.offset,
            to: token.offset + token.content.length,
            style: Object.entries(token.htmlStyle)
              .map(([name, value]) => `${name}:${value}`)
              .join(';')
          }
        ]
      : []
  )
}
