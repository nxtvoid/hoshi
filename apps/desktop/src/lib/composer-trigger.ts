export type ComposerTrigger = {
  kind: 'path' | 'command' | 'pullRequest' | 'skill' | 'note'
  /** What follows the `@`, `/`, `#`, `$` or `!`, up to the cursor. */
  query: string
  /** Where the token starts, at its `@`, `/`, `#`, `$` or `!`. */
  start: number
  /** Where the token ends, which may be past the cursor. */
  end: number
}

const SPACE = /\s/
/** Nothing yet, or a word: not `##`, nor `!=` or the `!` that ends a sentence. */
const WORD_QUERY = /^(?:[\p{L}\p{N}][\p{L}\p{N}_-]*)?$/u
/** Not `$5` or `$12.50`, which are prices. */
const SKILL_QUERY = /^(?:[A-Za-z][\w:-]*)?$/

/**
 * The `@path`, `#pull-request`, `$skill` or `!note` token the cursor is in, or the
 * `/command` the prompt starts with.
 */
export function detectTrigger(
  text: string,
  cursor: number
): ComposerTrigger | null {
  let start = cursor
  while (start > 0 && !SPACE.test(text[start - 1] ?? '')) start--
  let end = cursor
  while (end < text.length && !SPACE.test(text[end] ?? '')) end++
  const token = text.slice(start, cursor)

  if (token.startsWith('@')) {
    const query = token.slice(1).replace(/^"/, '')
    return { kind: 'path', query, start, end }
  }
  if (
    token.startsWith('/') &&
    !token.slice(1).includes('/') &&
    text.slice(0, start).trim() === ''
  ) {
    return { kind: 'command', query: token.slice(1), start, end }
  }
  const query = token.slice(1)
  if (token.startsWith('#') && WORD_QUERY.test(query)) {
    return { kind: 'pullRequest', query, start, end }
  }
  if (token.startsWith('!') && WORD_QUERY.test(query)) {
    return { kind: 'note', query, start, end }
  }
  if (token.startsWith('$') && SKILL_QUERY.test(query)) {
    return { kind: 'skill', query, start, end }
  }
  return null
}
