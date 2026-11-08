import { describe, expect, test } from 'bun:test'
import type { TranscriptRow } from '@/lib/tool-calls'
import type { ItemContent } from '@/bindings/ItemContent'
import {
  minimapCurrentIndex,
  minimapIndexAt,
  minimapItems,
  minimapTopPercent,
  plainPreview
} from '@/lib/transcript-minimap'

const row = (id: string, content: ItemContent): TranscriptRow => ({
  kind: 'item',
  item: { id, threadId: 't', createdAt: '', content }
})
const user = (id: string, text: string) =>
  row(id, { kind: 'userMessage', text, attachments: [] })
const assistant = (id: string, text: string) =>
  row(id, { kind: 'assistantMessage', text })

describe('minimapItems', () => {
  test('marks each user message with the last answer to it', () => {
    const rows: TranscriptRow[] = [
      assistant('a0', 'before anything'),
      user('u1', 'first'),
      assistant('a1', 'thinking out loud'),
      { kind: 'work', id: 'w', entries: [] },
      assistant('a2', 'final answer'),
      user('u2', 'second')
    ]
    expect(minimapItems(rows)).toEqual([
      { id: 'u1', userText: 'first', assistantText: 'final answer' },
      { id: 'u2', userText: 'second', assistantText: null }
    ])
  })
})

describe('minimapTopPercent', () => {
  test('spreads marks from the top to the bottom of the rail', () => {
    expect(minimapTopPercent(0, 3)).toBe(0)
    expect(minimapTopPercent(1, 3)).toBe(50)
    expect(minimapTopPercent(2, 3)).toBe(100)
    expect(minimapTopPercent(0, 1)).toBe(0)
  })
})

describe('minimapIndexAt', () => {
  test('picks the nearest mark and stays on the rail', () => {
    expect(minimapIndexAt(100, 100, 40, 5)).toBe(0)
    expect(minimapIndexAt(119, 100, 40, 5)).toBe(2)
    expect(minimapIndexAt(500, 100, 40, 5)).toBe(4)
    expect(minimapIndexAt(0, 100, 40, 5)).toBe(0)
    expect(minimapIndexAt(0, 100, 0, 5)).toBeNull()
  })
})

describe('minimapCurrentIndex', () => {
  const bounds = [
    { top: -500, bottom: -460 },
    { top: 200, bottom: 240 },
    { top: 900, bottom: 940 }
  ]

  test('prefers the first message on screen', () => {
    expect(minimapCurrentIndex(bounds, 0, 600)).toBe(1)
  })

  test('falls back to the message whose answer fills the screen', () => {
    expect(minimapCurrentIndex(bounds, 300, 800)).toBe(1)
    expect(minimapCurrentIndex(bounds, -400, 100)).toBe(0)
    expect(minimapCurrentIndex(bounds, -1000, -800)).toBeNull()
  })
})

describe('plainPreview', () => {
  test('drops markdown syntax and folds whitespace', () => {
    expect(
      plainPreview(
        '## Done\n\n- **Fixed** the `header`\n- See [docs](https://x.y)\n```ts\nconst a = 1\n```'
      )
    ).toBe('Done Fixed the header See docs const a = 1')
  })
})
