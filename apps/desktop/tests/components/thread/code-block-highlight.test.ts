import { describe, expect, test } from 'bun:test'
import { Schema } from '@tiptap/pm/model'
import { Transform } from '@tiptap/pm/transform'
import type { CodeToken } from '../../../src/lib/highlight'
import type { Node as ProseMirrorNode } from '@tiptap/pm/model'
import {
  type BlockTokens,
  codeBlocks,
  highlightedRuns,
  MAX_HIGHLIGHTED_CHARS,
  mapCodeBlocks,
  sameCodeBlocks
} from '../../../src/components/thread/code-block-highlight'

const schema = new Schema({
  nodes: {
    doc: { content: 'block+' },
    paragraph: { group: 'block', content: 'text*' },
    codeBlock: {
      group: 'block',
      content: 'text*',
      code: true,
      attrs: { language: { default: null } }
    },
    text: {}
  }
})

const paragraph = (text: string) =>
  schema.nodes.paragraph.create(null, text ? schema.text(text) : null)
const code = (text: string, language: string | null = 'ts') =>
  schema.nodes.codeBlock.create({ language }, text ? schema.text(text) : null)
const doc = (...blocks: ProseMirrorNode[]) =>
  schema.nodes.doc.create(null, blocks)

function counting() {
  const calls: { code: string; language: string }[] = []
  const tokenize = async (text: string, language: string) => {
    calls.push({ code: text, language })
    if (!language) return null
    return [
      { from: 0, to: text.length, style: `--shiki-light:${language}` }
    ] satisfies CodeToken[]
  }
  return { calls, tokenize }
}

describe('codeBlocks', () => {
  test('finds each code block and its position, skipping very long ones', () => {
    const document = doc(
      paragraph('intro'),
      code('const a = 1'),
      code('x'.repeat(MAX_HIGHLIGHTED_CHARS + 1)),
      code('let b')
    )

    expect(codeBlocks(document).map(({ pos }) => pos)).toEqual([
      7,
      20 + MAX_HIGHLIGHTED_CHARS + 3
    ])
  })
})

describe('highlightedRuns', () => {
  test('places each block’s tokens inside the block', async () => {
    const document = doc(paragraph('intro'), code('abc'), code('de', 'rs'))
    const { tokenize } = counting()

    const runs = await highlightedRuns(
      codeBlocks(document),
      new WeakMap(),
      tokenize
    )

    expect(runs).toEqual([
      { from: 8, to: 11, style: '--shiki-light:ts' },
      { from: 13, to: 15, style: '--shiki-light:rs' }
    ])
  })

  test('typing outside the code blocks tokenizes nothing and shifts the runs', async () => {
    const before = doc(paragraph('intro'), code('abc'), code('de'))
    const cache: BlockTokens = new WeakMap()
    const { calls, tokenize } = counting()
    await highlightedRuns(codeBlocks(before), cache, tokenize)

    const after = new Transform(before).insert(3, schema.text('!!')).doc
    const runs = await highlightedRuns(codeBlocks(after), cache, tokenize)

    expect(calls).toHaveLength(2)
    expect(runs.map(({ from, to }) => [from, to])).toEqual([
      [10, 13],
      [15, 17]
    ])
  })

  test('only the block whose code or language changed is tokenized again', async () => {
    const before = doc(code('abc'), code('de'))
    const cache: BlockTokens = new WeakMap()
    const { calls, tokenize } = counting()
    await highlightedRuns(codeBlocks(before), cache, tokenize)

    const typed = new Transform(before).insert(8, schema.text('f')).doc
    await highlightedRuns(codeBlocks(typed), cache, tokenize)
    const relabeled = new Transform(typed).setNodeMarkup(0, undefined, {
      language: 'py'
    }).doc
    await highlightedRuns(codeBlocks(relabeled), cache, tokenize)

    expect(calls).toEqual([
      { code: 'abc', language: 'ts' },
      { code: 'de', language: 'ts' },
      { code: 'def', language: 'ts' },
      { code: 'abc', language: 'py' }
    ])
  })

  test('a block without a known language stays plain', async () => {
    const document = doc(code('plain', null))
    const { tokenize } = counting()

    expect(
      await highlightedRuns(codeBlocks(document), new WeakMap(), tokenize)
    ).toEqual([])
  })

  test('a block whose highlighting failed is tried again', async () => {
    const document = doc(code('abc'))
    const cache: BlockTokens = new WeakMap()
    let attempts = 0
    const flaky = async (text: string) => {
      attempts += 1
      if (attempts === 1) throw new Error('not loaded')
      return [{ from: 0, to: text.length, style: 'color:red' }]
    }

    await expect(
      highlightedRuns(codeBlocks(document), cache, flaky)
    ).rejects.toThrow()
    const runs = await highlightedRuns(codeBlocks(document), cache, flaky)

    expect(attempts).toBe(2)
    expect(runs).toEqual([{ from: 1, to: 4, style: 'color:red' }])
  })
})

describe('mapCodeBlocks', () => {
  test('follows the blocks through a change before them', () => {
    const before = doc(paragraph('intro'), code('abc'))
    const transform = new Transform(before).insert(3, schema.text('!!'))

    expect(
      sameCodeBlocks(
        mapCodeBlocks(codeBlocks(before), transform.mapping),
        codeBlocks(transform.doc)
      )
    ).toBe(true)
  })

  test('drops a deleted block, so the rest still match', () => {
    const before = doc(code('abc'), code('de'))
    const transform = new Transform(before).delete(0, 5)

    const mapped = mapCodeBlocks(codeBlocks(before), transform.mapping)

    expect(mapped).toHaveLength(1)
    expect(sameCodeBlocks(mapped, codeBlocks(transform.doc))).toBe(true)
  })

  test('an edited block no longer matches', () => {
    const before = doc(code('abc'))
    const transform = new Transform(before).insert(2, schema.text('x'))

    expect(
      sameCodeBlocks(
        mapCodeBlocks(codeBlocks(before), transform.mapping),
        codeBlocks(transform.doc)
      )
    ).toBe(false)
  })
})
