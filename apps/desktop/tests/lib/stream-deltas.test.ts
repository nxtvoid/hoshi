import { describe, expect, test } from 'bun:test'
import type { StreamingText } from '../../src/lib/tool-calls'
import type { StreamDelta } from '../../src/bindings/StreamDelta'
import {
  applyDeltas,
  bufferDelta,
  type LiveText
} from '../../src/lib/stream-deltas'

const delta = (
  itemId: string,
  kind: StreamDelta['kind'],
  text: string
): StreamDelta => ({ threadId: 't', itemId, kind, delta: text })

const EMPTY: LiveText = { streaming: {}, toolOutput: {} }

describe('bufferDelta', () => {
  test('appends text in order', () => {
    const buffer = new Map<string, StreamingText>()
    bufferDelta(buffer, delta('a', 'text', 'Hel'))
    bufferDelta(buffer, delta('a', 'text', 'lo'))
    bufferDelta(buffer, delta('b', 'reasoning', 'Hmm'))
    expect(buffer.get('a')).toEqual({ kind: 'text', text: 'Hello' })
    expect(buffer.get('b')).toEqual({ kind: 'reasoning', text: 'Hmm' })
  })

  test('keeps only the latest tool output', () => {
    const buffer = new Map<string, StreamingText>()
    bufferDelta(buffer, delta('call', 'toolOutput', 'line 1'))
    bufferDelta(buffer, delta('call', 'toolOutput', 'line 1\nline 2'))
    expect(buffer.get('call')).toEqual({
      kind: 'toolOutput',
      text: 'line 1\nline 2'
    })
  })
})

describe('applyDeltas', () => {
  test('appends buffered text to what already streamed', () => {
    const live: LiveText = {
      streaming: { a: { kind: 'text', text: 'Hello' } },
      toolOutput: {}
    }
    const buffer = new Map<string, StreamingText>([
      ['a', { kind: 'text', text: ', world' }],
      ['b', { kind: 'reasoning', text: 'Thinking' }]
    ])
    const next = applyDeltas(live, buffer)
    expect(next.streaming).toEqual({
      a: { kind: 'text', text: 'Hello, world' },
      b: { kind: 'reasoning', text: 'Thinking' }
    })
    expect(next.toolOutput).toBe(live.toolOutput)
    expect(live.streaming.a?.text).toBe('Hello')
  })

  test('puts tool output in its own record', () => {
    const buffer = new Map<string, StreamingText>([
      ['call', { kind: 'toolOutput', text: 'done' }]
    ])
    const next = applyDeltas(EMPTY, buffer)
    expect(next.toolOutput).toEqual({ call: 'done' })
    expect(next.streaming).toBe(EMPTY.streaming)
  })

  test('returns the same value when nothing changes', () => {
    const live: LiveText = { streaming: {}, toolOutput: { call: 'same' } }
    const buffer = new Map<string, StreamingText>([
      ['call', { kind: 'toolOutput', text: 'same' }]
    ])
    expect(applyDeltas(live, buffer)).toBe(live)
    expect(applyDeltas(live, new Map())).toBe(live)
  })
})
