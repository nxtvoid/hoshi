import { describe, expect, test } from 'bun:test'
import type { Thread } from '../../src/bindings/Thread'
import {
  finishedTurns,
  projectSignal,
  threadNotices
} from '../../src/state/thread-signals'

const thread = (id: string, changes: Partial<Thread> = {}): Thread => ({
  id,
  projectId: 'p',
  title: id,
  status: 'idle',
  branch: null,
  worktreePath: null,
  sessionId: null,
  model: null,
  effort: null,
  fastMode: false,
  ultracode: false,
  accessMode: 'acceptEdits',
  planMode: false,
  activeTurnId: null,
  archived: false,
  pinned: false,
  worktreeReleased: false,
  worktreeAdopted: false,
  createdAt: '2026-11-01T00:00:00.000Z',
  updatedAt: '2026-11-01T00:00:00.000Z',
  ...changes
})

describe('finishedTurns', () => {
  test('finds turns that went from working to idle', () => {
    const before = [
      thread('done', { status: 'working' }),
      thread('failed', { status: 'working' }),
      thread('still', { status: 'working' }),
      thread('idle')
    ]
    const after = [
      thread('done'),
      thread('failed', { status: 'error' }),
      thread('still', { status: 'working' }),
      thread('idle')
    ]
    expect(finishedTurns(before, after)).toEqual(['done'])
  })
})

describe('projectSignal', () => {
  test('ranks failures over questions over finished turns over work', () => {
    const threads = [
      thread('a', { status: 'working' }),
      thread('b'),
      thread('c', { status: 'waitingForInput' })
    ]
    expect(projectSignal(threads, 'p', { b: true })).toBe('waiting')
    expect(projectSignal(threads.slice(0, 2), 'p', { b: true })).toBe('done')
    expect(projectSignal(threads.slice(0, 2), 'p', {})).toBe('working')
    expect(
      projectSignal([...threads, thread('d', { status: 'error' })], 'p', {})
    ).toBe('error')
  })

  test('only looks at the project and skips archived threads', () => {
    const threads = [
      thread('a', { projectId: 'q', status: 'error' }),
      thread('b', { archived: true, status: 'waitingForInput' })
    ]
    expect(projectSignal(threads, 'p', {})).toBeNull()
  })
})

describe('threadNotices', () => {
  test('tells about turns that end, fail or ask something', () => {
    const before = [
      thread('a', { status: 'working' }),
      thread('b', { status: 'working' }),
      thread('c', { status: 'working' }),
      thread('d', { status: 'idle' }),
      thread('e', { status: 'error' })
    ]
    const after = [
      thread('a', { status: 'idle' }),
      thread('b', { status: 'error' }),
      thread('c', { status: 'waitingForInput' }),
      thread('d', { status: 'working' }),
      thread('e', { status: 'error' }),
      thread('new', { status: 'waitingForInput' })
    ]

    expect(threadNotices(before, after)).toEqual([
      { threadId: 'a', kind: 'finished' },
      { threadId: 'b', kind: 'failed' },
      { threadId: 'c', kind: 'waiting' }
    ])
  })

  test('an answered question going idle is no news', () => {
    expect(
      threadNotices(
        [thread('a', { status: 'waitingForInput' })],
        [thread('a', { status: 'idle' })]
      )
    ).toEqual([])
  })
})
