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
