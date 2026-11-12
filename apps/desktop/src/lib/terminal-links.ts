/** A web address in a line of terminal output, as offsets into the line. */
export type TerminalLink = { url: string; start: number; end: number }

/** A buffer line as xterm gives it. */
export type BufferLine = {
  readonly isWrapped: boolean
  translateToString(trimRight?: boolean): string
}

/** One buffer line's part of a line the terminal wrapped, with its offsets into the whole. */
export type Segment = { lineNumber: number; start: number; end: number }

export type WrappedLine = { text: string; segments: Segment[] }

const URL_PATTERN = /https?:\/\/[^\s"'`<>]+/giu
const TRAILING_PUNCTUATION = /[.,;!?]+$/

/** Drops sentence punctuation and closing brackets the address didn't open. */
function trimEnd(url: string) {
  let output = url.replace(TRAILING_PUNCTUATION, '')
  for (const [open, close] of [
    ['(', ')'],
    ['[', ']'],
    ['{', '}']
  ]) {
    while (output.endsWith(close)) {
      const opens = output.split(open).length - 1
      const closes = output.split(close).length - 1
      if (opens >= closes) break
      output = output.slice(0, -1)
    }
  }
  return output
}

export function findLinks(line: string): TerminalLink[] {
  const links: TerminalLink[] = []
  for (const match of line.matchAll(URL_PATTERN)) {
    const url = trimEnd(match[0])
    if (url.length > 'https://'.length) {
      links.push({ url, start: match.index, end: match.index + url.length })
    }
  }
  return links
}

/**
 * The whole line that buffer line `lineNumber` (1-based) is part of, across the lines the
 * terminal wrapped it over.
 */
export function wrappedLine(
  lineNumber: number,
  getLine: (index: number) => BufferLine | undefined
): WrappedLine | null {
  let first = lineNumber
  let line = getLine(first - 1)
  if (!line) return null
  while (first > 1 && line.isWrapped) {
    const previous = getLine(first - 2)
    if (!previous) return null
    first -= 1
    line = previous
  }
  const segments: Segment[] = []
  let text = ''
  for (let number = first; ; number += 1) {
    const current = getLine(number - 1)
    if (!current) break
    const continues = getLine(number)?.isWrapped === true
    const part = current.translateToString(!continues)
    segments.push({
      lineNumber: number,
      start: text.length,
      end: text.length + part.length
    })
    text += part
    if (!continues) break
  }
  return { text, segments }
}

/** Where offset `index` of a wrapped line is: its buffer line and 1-based column. */
export function cellAt(segments: Segment[], index: number) {
  const segment =
    segments.find((part) => index >= part.start && index < part.end) ??
    segments[segments.length - 1]
  return { x: index - segment.start + 1, y: segment.lineNumber }
}
