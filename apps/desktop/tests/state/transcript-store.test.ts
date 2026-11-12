import { beforeEach, describe, expect, test } from 'bun:test'
import type { Event } from '../../src/bindings/Event'
import type { ThreadItem } from '../../src/bindings/ThreadItem'
import { useTranscriptStore } from '../../src/state/transcript-store'

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

const message = (id: string, text: string): ThreadItem => ({
  id,
  threadId: 't',
  createdAt: '2026-11-08T10:00:00.000Z',
  content: { kind: 'assistantMessage', text }
})

const added = (item: ThreadItem): Event => ({ type: 'itemAdded', item })

const store = () => useTranscriptStore.getState()

const write = (itemId: string, delta: string, threadId = 't') =>
  store().applyDelta({ threadId, itemId, kind: 'text', delta })

describe('transcript deltas', () => {
  beforeEach(() => {
    useTranscriptStore.setState({
      threadId: 't',
      loading: false,
      items: [],
      streaming: {},
      toolOutput: {}
    })
  })

  test('shows deltas together once the frame comes', async () => {
    write('a', 'He')
    write('a', 'y')
    expect(store().streaming).toEqual({})
    await wait(80)
    expect(store().streaming).toEqual({ a: { kind: 'text', text: 'Hey' } })
  })

  test('ignores deltas of another thread', async () => {
    write('a', 'He', 'x')
    await wait(80)
    expect(store().streaming).toEqual({})
  })

  test('the finished item drops its pending deltas', async () => {
    write('a', 'He')
    store().applyEvent(added(message('a', 'Hey')))
    expect(store().items).toEqual([message('a', 'Hey')])
    expect(store().streaming).toEqual({})
    await wait(80)
    expect(store().streaming).toEqual({})
  })

  test('other events show pending deltas first', () => {
    write('b', 'Hi')
    store().applyEvent(added(message('a', 'Done')))
    expect(store().streaming).toEqual({ b: { kind: 'text', text: 'Hi' } })
  })

  test('tool output stays out of the streaming text', async () => {
    store().applyDelta({
      threadId: 't',
      itemId: 'call',
      kind: 'toolOutput',
      delta: 'one'
    })
    store().applyDelta({
      threadId: 't',
      itemId: 'call',
      kind: 'toolOutput',
      delta: 'one\ntwo'
    })
    await wait(80)
    expect(store().toolOutput).toEqual({ call: 'one\ntwo' })
    expect(store().streaming).toEqual({})
  })

  test('the end of the turn clears what still streams', () => {
    write('a', 'He')
    store().applyDelta({
      threadId: 't',
      itemId: 'call',
      kind: 'toolOutput',
      delta: 'out'
    })
    store().applyEvent({
      type: 'turnCompleted',
      threadId: 't',
      turnId: 'turn',
      outcome: { type: 'completed' },
      at: '2026-11-08T10:00:01.000Z'
    })
    expect(store().streaming).toEqual({})
    expect(store().toolOutput).toEqual({})
  })
})

describe('switching threads', () => {
  const pages: Record<string, ThreadItem[]> = {}
  const asked: string[] = []
  const streamed: (string | null)[] = []
  let slow = false
  const item = (threadId: string, id: string): ThreadItem => ({
    ...message(id, id),
    threadId
  })

  beforeEach(() => {
    asked.length = 0
    streamed.length = 0
    slow = false
    const scope = globalThis as { window?: Record<string, unknown> }
    scope.window ??= {}
    scope.window.__TAURI_INTERNALS__ = {
      invoke: async (command: string, args: { threadId: string }) => {
        if (command === 'stream_thread') {
          streamed.push(args.threadId)
          return null
        }
        asked.push(args.threadId)
        if (slow) await wait(600)
        return { items: pages[args.threadId] ?? [], hasOlder: false }
      }
    }
  })

  test('a thread opened lately shows at once, with what came meanwhile', async () => {
    pages.k1 = [item('k1', 'a1')]
    pages.k2 = [item('k2', 'b1')]
    await store().load('k1')
    await store().load('k2')
    store().applyEvent(added(item('k1', 'a2')))
    void store().load('k1')

    expect(store().loading).toBe(false)
    expect(store().items.map((kept) => kept.id)).toEqual(['a1', 'a2'])
    expect(asked).toEqual(['k1', 'k2'])
  })

  test('only the thread shown streams, asked once per switch', async () => {
    pages.s1 = [item('s1', 'g1')]
    await store().load('s1')
    await store().load('s1')
    await store().load(null)

    expect(streamed).toEqual(['s1', null])
  })

  test("the previous thread stays until the next one's transcript is in", async () => {
    pages.k3 = [item('k3', 'c1')]
    pages.k4 = [item('k4', 'd1')]
    await store().load('k3')
    const loading = store().load('k4')

    expect(store().loading).toBe(true)
    expect(store().items.map((kept) => kept.id)).toEqual(['c1'])
    await loading
    expect(store().items.map((kept) => kept.id)).toEqual(['d1'])
  })

  test('the items stay credited to the thread they belong to while the next one loads', async () => {
    pages.m1 = [item('m1', 'h1')]
    pages.m2 = [item('m2', 'i1')]
    await store().load('m1')
    const loading = store().load('m2')

    expect(store().shownId).toBe('m1')
    await loading
    expect(store().shownId).toBe('m2')
    expect(store().recent.slice(0, 2)).toEqual(['m2', 'm1'])
  })

  test('the threads shown lately are a short list, latest first, without deleted ones', async () => {
    for (const id of ['r1', 'r2', 'r3', 'r4', 'r5']) {
      pages[id] = [item(id, `${id}-a`)]
      await store().load(id)
    }
    await store().load('r3')
    expect(store().recent).toEqual(['r3', 'r5', 'r4', 'r2'])

    store().applyEvent({ type: 'threadDeleted', threadId: 'r4' })
    expect(store().recent).toEqual(['r3', 'r5', 'r2'])
  })

  test('a slow transcript blanks the previous one after a moment', async () => {
    pages.k5 = [item('k5', 'e1')]
    pages.k6 = [item('k6', 'f1')]
    await store().load('k5')
    slow = true
    const loading = store().load('k6')
    await wait(450)

    expect(store().items).toEqual([])
    await loading
    expect(store().items.map((kept) => kept.id)).toEqual(['f1'])
  })
})
