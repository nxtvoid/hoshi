import { describe, expect, test } from 'bun:test'
import { splitMarkdownBlocks } from '../../src/lib/markdown-blocks'

describe('splitMarkdownBlocks', () => {
  test('cuts at blank lines between blocks, keeping every character', () => {
    const text = '# Title\n\nFirst paragraph\nstill first.\n\nSecond'
    const blocks = splitMarkdownBlocks(text)

    expect(blocks).toEqual([
      '# Title\n\n',
      'First paragraph\nstill first.\n\n',
      'Second'
    ])
    expect(blocks?.join('')).toBe(text)
  })

  test('keeps a fenced block whole, blank lines and all', () => {
    const text = 'Before\n\n```ts\nconst a = 1\n\nconst b = 2\n```\n\nAfter'

    expect(splitMarkdownBlocks(text)).toEqual([
      'Before\n\n',
      '```ts\nconst a = 1\n\nconst b = 2\n```\n\n',
      'After'
    ])
  })

  test('keeps a fence still open as the last block', () => {
    expect(splitMarkdownBlocks('Intro\n\n```\npartial\n\nmore')).toEqual([
      'Intro\n\n',
      '```\npartial\n\nmore'
    ])
  })

  test('keeps a loose list and its indented text together', () => {
    const text = '- one\n\n- two\n\n  more of two\n\nAfter'

    expect(splitMarkdownBlocks(text)).toEqual([
      '- one\n\n- two\n\n  more of two\n\n',
      'After'
    ])
  })

  test('leaves whole the text that syntax ties across blocks', () => {
    expect(splitMarkdownBlocks('See [docs].\n\n[docs]: https://x.y')).toBeNull()
    expect(splitMarkdownBlocks('<details>\n\nBody\n\n</details>')).toBeNull()
    expect(splitMarkdownBlocks('A note[^1]\n\n[^1]: Here')).toBeNull()
    expect(splitMarkdownBlocks('One\r\n\r\nTwo')).toBeNull()
  })
})
