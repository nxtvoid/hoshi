import { describe, expect, test } from 'bun:test'
import {
  type BufferLine,
  cellAt,
  findLinks,
  wrappedLine
} from '../../src/lib/terminal-links'

describe('findLinks', () => {
  test('finds addresses and leaves sentence punctuation out', () => {
    expect(findLinks('ready on http://localhost:5173/, open it.')).toEqual([
      { url: 'http://localhost:5173/', start: 9, end: 31 }
    ])
  })

  test('keeps parentheses the address opened', () => {
    const line = '(see https://en.wikipedia.org/wiki/Foo_(bar))'
    expect(findLinks(line).map((link) => link.url)).toEqual([
      'https://en.wikipedia.org/wiki/Foo_(bar)'
    ])
  })

  test('finds each of several addresses', () => {
    expect(
      findLinks('a https://a.dev b http://b.dev/x?y=1').map((link) => link.url)
    ).toEqual(['https://a.dev', 'http://b.dev/x?y=1'])
  })

  test('a bare scheme is no link', () => {
    expect(findLinks('use https:// here')).toEqual([])
  })
})

describe('wrappedLine', () => {
  const buffer: BufferLine[] = [
    { isWrapped: false, translateToString: () => 'first line' },
    { isWrapped: false, translateToString: () => 'open https://exam' },
    { isWrapped: true, translateToString: () => 'ple.com/a/b now' }
  ]
  const getLine = (index: number) => buffer[index]

  test('joins the lines the terminal wrapped, from any of them', () => {
    const fromFirst = wrappedLine(2, getLine)
    const fromSecond = wrappedLine(3, getLine)
    expect(fromFirst?.text).toBe('open https://example.com/a/b now')
    expect(fromSecond).toEqual(fromFirst)
  })

  test('maps a link back to the cells it spans', () => {
    const line = wrappedLine(3, getLine)
    if (!line) throw new Error('no line')
    const [link] = findLinks(line.text)
    expect(link.url).toBe('https://example.com/a/b')
    expect(cellAt(line.segments, link.start)).toEqual({ x: 6, y: 2 })
    expect(cellAt(line.segments, link.end - 1)).toEqual({ x: 11, y: 3 })
  })
})
