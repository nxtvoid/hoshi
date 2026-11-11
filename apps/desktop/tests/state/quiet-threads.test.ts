import { describe, expect, test } from 'bun:test'
import type { Thread } from '../../src/bindings/Thread'
import { quietThreads } from '../../src/state/quiet-threads'

const thread = (id: string, change: Partial<Thread> = {}): Thread =>
  ({
    id,
    status: 'idle',
    title: id,
    updatedAt: '2026-11-11T12:00:00.000Z',
    ...change
  }) as Thread

describe('quietThreads', () => {
  test("a working thread's items don't make a new list", () => {
    const shown = quietThreads(null, [thread('a', { status: 'working' })])
    const next = quietThreads(shown, [
      thread('a', { status: 'working', updatedAt: '2026-11-11T12:00:05.000Z' })
    ])

    expect(next).toBe(shown)
  })

  test('anything else that changes shows', () => {
    const shown = quietThreads(null, [thread('a', { status: 'working' })])
    const done = quietThreads(shown, [thread('a', { status: 'idle' })])
    const renamed = quietThreads(shown, [
      thread('a', { status: 'working', title: 'b' })
    ])
    const idle = quietThreads(null, [thread('b')])
    const touched = quietThreads(idle, [
      thread('b', { updatedAt: '2026-11-11T12:00:05.000Z' })
    ])

    expect(done[0]?.status).toBe('idle')
    expect(renamed[0]?.title).toBe('b')
    expect(touched).not.toBe(idle)
  })

  test('a new or removed thread makes a new list', () => {
    const shown = quietThreads(null, [thread('a')])
    expect(quietThreads(shown, [thread('a'), thread('b')])).toHaveLength(2)
    expect(quietThreads(shown, [])).toEqual([])
  })
})
