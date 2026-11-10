import { expect, test } from 'bun:test'
import { subagentRows } from '../../src/lib/subagent-rows'
import type { ThreadItem } from '../../src/bindings/ThreadItem'
import type { ToolCallItem, TranscriptRow } from '../../src/lib/tool-calls'

const at = (second: number) =>
  new Date(Date.UTC(2026, 10, 10, 10, 0, second)).toISOString()

function toolCall(id: string, threadId: string, second: number): ToolCallItem {
  return {
    id,
    threadId,
    createdAt: at(second),
    content: {
      kind: 'toolCall',
      name: id === 'call' ? 'Agent' : 'Read',
      input: '{}',
      output: 'done',
      isError: false,
      detail: null,
      outputOmitted: false,
      interrupted: false
    }
  }
}

function said(id: string, text: string, second: number): ThreadItem {
  return {
    id,
    threadId: 't:call',
    createdAt: at(second),
    content: { kind: 'assistantMessage', text }
  }
}

const call = toolCall('call', 't', 0)

function shape(rows: TranscriptRow[]) {
  return rows.map((row) =>
    row.kind === 'item' ? row.item.id : `${row.kind}(${row.id})`
  )
}

test('a finished subagent folds its work and ends on its report', () => {
  const rows = subagentRows(
    call,
    [toolCall('read', 't:call', 2), said('answer', 'Two pages.', 40)],
    'Two pages.',
    false
  )

  expect(shape(rows)).toEqual(['fold(fold:call:asked)', 'answer'])
  const [fold] = rows
  expect(fold?.kind === 'fold' && fold.durationMs).toBe(40_000)
})

test('a report its last message does not say comes after it', () => {
  const rows = subagentRows(
    call,
    [toolCall('read', 't:call', 2), said('answer', 'Looking.', 3)],
    'Two pages.',
    false
  )

  const last = rows.at(-1)
  expect(last?.kind === 'item' && last.item.id).toBe('call:report')
  expect(
    last?.kind === 'item' &&
      last.item.content.kind === 'assistantMessage' &&
      last.item.content.text
  ).toBe('Two pages.')
})

test('a running subagent keeps its work open', () => {
  const rows = subagentRows(call, [toolCall('read', 't:call', 2)], null, true)

  expect(rows.map((row) => row.kind)).toEqual(['work'])
})

test('a subagent that did nothing has no rows', () => {
  expect(subagentRows(call, [], null, false)).toEqual([])
})
