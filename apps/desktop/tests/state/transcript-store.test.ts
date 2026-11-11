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
  let slow = false
  const item = (threadId: string, id: string): ThreadItem => ({
    ...message(id, id),
    threadId
  })

  beforeEach(() => {
    asked.length = 0
    slow = false
    const scope = globalThis as { window?: Record<string, unknown> }
    scope.window ??= {}
    scope.window.__TAURI_INTERNALS__ = {
      invoke: async (_command: string, args: { threadId: string }) => {
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
