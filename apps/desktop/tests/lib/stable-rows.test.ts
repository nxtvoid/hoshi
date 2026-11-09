import { describe, expect, test } from 'bun:test'
import type { ThreadItem } from '../../src/bindings/ThreadItem'
import { stableRows } from '../../src/lib/stable-rows'
import {
  foldTurns,
  groupTranscript,
  withStreaming,
  type TranscriptRow
} from '../../src/lib/tool-calls'

const item = (
  id: string,
  content: ThreadItem['content'],
  createdAt = ''
): ThreadItem => ({ id, threadId: 't', createdAt, content })

const user = (id: string, createdAt: string) =>
  item(
    id,
    { kind: 'userMessage', text: id, attachments: [], steer: false },
    createdAt
  )
const answer = (id: string, text = id, createdAt = '') =>
  item(id, { kind: 'assistantMessage', text }, createdAt)
const read = (id: string, output: string | null = '') =>
  item(id, {
    kind: 'toolCall',
    name: 'Read',
    input: '{}',
    output,
    isError: false,
    detail: null,
    outputOmitted: false,
    interrupted: false
  })

const rowsOf = (items: ThreadItem[], streaming = {}, working = true) =>
  foldTurns(groupTranscript(withStreaming(items, streaming)), working)

describe('stableRows', () => {
  test('keeps the rows that did not change, and the list when none did', () => {
    const items = [user('u1', ''), read('r'), answer('a')]
    const first = rowsOf(items)
    const again = stableRows(first, rowsOf([...items]))

    expect(again).toBe(first)

    const edited = [...items.slice(0, 2), answer('a', 'changed')]
    const next = stableRows(first, rowsOf(edited))
    expect(next).not.toBe(first)
    expect(next[0]).toBe(first[0] as TranscriptRow)
    expect(next[1]).toBe(first[1] as TranscriptRow)
    expect(next[2]).not.toBe(first[2])
  })

  test('keeps text still streaming while it does not change', () => {
    const items = [user('u1', ''), read('r')]
    const first = rowsOf(items, {
      a: { kind: 'text', text: 'Hel' },
      t: { kind: 'reasoning', text: 'hm' }
    })
    const same = stableRows(
      first,
      rowsOf(items, {
        a: { kind: 'text', text: 'Hel' },
        t: { kind: 'reasoning', text: 'hm' }
      })
    )
    expect(same).toBe(first)

    const grown = stableRows(
      first,
      rowsOf(items, {
        a: { kind: 'text', text: 'Hello' },
        t: { kind: 'reasoning', text: 'hm' }
      })
    )
    expect(grown[1]).toBe(first[1] as TranscriptRow)
    expect(grown[2]).not.toBe(first[2])
  })

  test('keeps a fold whose work did not change, once its answer streams on after it', () => {
    const items = [
      user('u1', '2026-11-08T10:00:00Z'),
      read('r'),
      answer('a1', 'a1', '2026-11-08T10:00:05Z'),
      user('u2', '2026-11-08T10:01:00Z'),
      read('s', null)
    ]
    const first = rowsOf(items)
    const fold = first[1]
    expect(fold?.kind).toBe('fold')

    const done = [...items.slice(0, 4), read('s')]
    const next = stableRows(first, rowsOf(done))
    expect(next[1]).toBe(fold as TranscriptRow)
    expect(next[4]).not.toBe(first[4])
  })
})
