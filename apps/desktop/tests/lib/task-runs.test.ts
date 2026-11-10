import { expect, test } from 'bun:test'
import type { ThreadItem } from '../../src/bindings/ThreadItem'
import { foldTaskRuns, taskOutcome } from '../../src/lib/task-runs'
import type { TranscriptRow } from '../../src/lib/tool-calls'

const at = (second: number) =>
  new Date(Date.UTC(2026, 10, 10, 10, 0, second)).toISOString()

function row(item: ThreadItem): TranscriptRow {
  return { kind: 'item', item }
}

function user(id: string, second: number): TranscriptRow {
  return row({
    id,
    threadId: 't',
    createdAt: at(second),
    content: {
      kind: 'userMessage',
      text: id,
      attachments: [],
      steer: false,
      taskResult: null,
      pullRequestUpdate: null,
      rewind: null
    }
  })
}

function result(
  id: string,
  task: string,
  second: number,
  status = 'completed'
): TranscriptRow {
  return row({
    id,
    threadId: 't',
    createdAt: at(second),
    content: {
      kind: 'userMessage',
      text: `Delegated task ${task} (${task}) [${status}]:\n${task} is done`,
      attachments: [],
      steer: false,
      taskResult: task,
      pullRequestUpdate: null,
      rewind: null
    }
  })
}

function said(id: string, second: number): TranscriptRow {
  return row({
    id,
    threadId: 't',
    createdAt: at(second),
    content: { kind: 'assistantMessage', text: id }
  })
}

function delegated(id: string, task: string): TranscriptRow {
  return row({
    id,
    threadId: 't',
    createdAt: at(1),
    content: {
      kind: 'toolCall',
      name: 'mcp__hoshi__delegate_task',
      input: JSON.stringify({ title: task }),
      output: JSON.stringify({ taskId: task }),
      isError: false,
      detail: null,
      outputOmitted: false,
      interrupted: false
    }
  })
}

const worked: TranscriptRow = {
  kind: 'fold',
  id: 'fold:u1',
  rows: [delegated('d1', 'a'), delegated('d2', 'b')],
  ending: 'answered',
  after: 'start',
  durationMs: 2000
}

const rows = [
  user('u1', 0),
  worked,
  said('started', 2),
  result('ra', 'a', 30),
  said('a answered', 31),
  result('rb', 'b', 90, 'failed'),
  said('summary', 92)
]

const none = () => false
const stillB = (task: string) => task === 'b'

const shape = (shown: TranscriptRow[]) =>
  shown.map((shownRow) =>
    shownRow.kind === 'item'
      ? shownRow.note
        ? `${shownRow.item.id} (${shownRow.note})`
        : shownRow.item.id
      : shownRow.kind === 'fold'
        ? { fold: shape(shownRow.rows), durationMs: shownRow.durationMs }
        : shownRow.id
  )

test('a task answer tells how the task ended', () => {
  expect(taskOutcome('Delegated task a (A) [completed]:\nDone')).toBe(
    'completed'
  )
  expect(taskOutcome('Delegated task a (A) [failed]:\nNo')).toBe('failed')
  expect(taskOutcome('Delegated task a (A) [stopped]:\n')).toBe('stopped')
})

test('while a task still runs, its answers hide and the replies read as notes', () => {
  const shown = foldTaskRuns(rows.slice(0, 5), stillB)

  expect(shape(shown)).toEqual([
    'u1',
    { fold: ['d1', 'd2'], durationMs: 2000 },
    'started',
    'a answered (completed)'
  ])
})

test('once every task answered, the work folds behind its last answer', () => {
  const shown = foldTaskRuns(rows, none)

  expect(shape(shown)).toEqual([
    'u1',
    {
      fold: ['d1', 'd2', 'started', 'a answered (completed)'],
      durationMs: 92000
    },
    'summary'
  ])
})

test('the agent still working on the last answer keeps it all open', () => {
  const shown = foldTaskRuns(rows, stillB)

  expect(shape(shown).at(-1)).toBe('summary (failed)')
})

test('the work behind each reply waits for the end, inside the fold', () => {
  const replyWork: TranscriptRow = { ...worked, id: 'fold:ra', rows: [] }
  const withWork = [...rows.slice(0, 4), replyWork, ...rows.slice(4)]

  expect(shape(foldTaskRuns(withWork.slice(0, 6), stillB))).toEqual([
    'u1',
    { fold: ['d1', 'd2'], durationMs: 2000 },
    'started',
    'a answered (completed)'
  ])
  expect(shape(foldTaskRuns(withWork, none))[1]).toEqual({
    fold: ['d1', 'd2', 'started', 'a answered (completed)'],
    durationMs: 92000
  })
})

test('once the tasks answered, the reply being written already stands alone', () => {
  const thinking: TranscriptRow = {
    kind: 'work',
    id: 'w1',
    entries: []
  }
  const writing = [...rows.slice(0, 6), thinking]

  expect(shape(foldTaskRuns(writing, none))).toEqual([
    'u1',
    {
      fold: ['d1', 'd2', 'started', 'a answered (completed)'],
      durationMs: 90000
    },
    'w1'
  ])
  expect(shape(foldTaskRuns([...writing, said('summary', 92)], none))).toEqual([
    'u1',
    {
      fold: ['d1', 'd2', 'started', 'a answered (completed)', 'w1'],
      durationMs: 92000
    },
    'summary'
  ])
})

test('what settled stays folded while the agent delegates again', () => {
  const first = [
    user('u1', 0),
    delegated('d1', 'a'),
    said('started', 2),
    result('ra', 'a', 10)
  ]
  expect(shape(foldTaskRuns(first, none))).toEqual([
    'u1',
    { fold: ['d1', 'started'], durationMs: 10000 }
  ])

  const second = [...first, delegated('d2', 'b'), said('got a', 12)]
  expect(shape(foldTaskRuns(second, stillB))).toEqual([
    'u1',
    { fold: ['d1', 'started'], durationMs: 1000 },
    'd2',
    'got a'
  ])

  const told = [...first, said('got a', 12), delegated('d2', 'b')]
  expect(shape(foldTaskRuns(told, stillB))).toEqual([
    'u1',
    { fold: ['d1', 'started'], durationMs: 12000 },
    'got a',
    'd2'
  ])

  const over = [...second, result('rb', 'b', 20), said('both', 22)]
  expect(shape(foldTaskRuns(over, none))).toEqual([
    'u1',
    {
      fold: ['d1', 'started', 'd2', 'got a (completed)'],
      durationMs: 22000
    },
    'both'
  ])
})

test('of the replies after the last answer, only the latest stays in view', () => {
  const shown = foldTaskRuns([...rows, said('all of it', 95)], none)

  expect(shape(shown)).toEqual([
    'u1',
    {
      fold: [
        'd1',
        'd2',
        'started',
        'a answered (completed)',
        'summary (failed)'
      ],
      durationMs: 95000
    },
    'all of it'
  ])
})

test('work without delegated tasks stays as it is', () => {
  const plain = [user('u1', 0), worked, said('done', 3)]
  expect(foldTaskRuns(plain, none)).toEqual(plain)
})

test('each message of the user starts its own stretch', () => {
  const shown = foldTaskRuns(
    [...rows, user('u2', 100), said('next', 101)],
    none
  )

  expect(shape(shown).slice(-3)).toEqual(['summary', 'u2', 'next'])
})

test('a stretch stays open while its own tasks run, after the user moved on', () => {
  const shown = foldTaskRuns(
    [...rows.slice(0, 5), user('u2', 40), said('next', 41)],
    stillB
  )

  expect(shape(shown)).toEqual([
    'u1',
    { fold: ['d1', 'd2'], durationMs: 2000 },
    'started',
    'a answered (completed)',
    'u2',
    'next'
  ])
})

test('what the agent tells the user between task answers is no note', () => {
  const steer = row({
    id: 'steer',
    threadId: 't',
    createdAt: at(32),
    content: {
      kind: 'userMessage',
      text: 'how is it going?',
      attachments: [],
      steer: true,
      taskResult: null,
      pullRequestUpdate: null,
      rewind: null
    }
  })
  const shown = foldTaskRuns(
    [...rows.slice(0, 5), steer, said('going well', 33)],
    stillB
  )

  expect(shape(shown).slice(-3)).toEqual([
    'a answered (completed)',
    'steer',
    'going well'
  ])

  const over = foldTaskRuns(
    [
      ...rows.slice(0, 5),
      steer,
      said('going well', 33),
      result('rb', 'b', 90),
      said('summary', 92)
    ],
    none
  )
  expect(shape(over)).toEqual([
    'u1',
    {
      fold: ['d1', 'd2', 'started', 'a answered (completed)'],
      durationMs: 32000
    },
    'steer',
    'going well',
    'summary'
  ])
})

test('each reply goes with the oldest answer or message still without one', () => {
  const steer = row({
    id: 'steer',
    threadId: 't',
    createdAt: at(31),
    content: {
      kind: 'userMessage',
      text: 'how are you?',
      attachments: [],
      steer: true,
      taskResult: null,
      pullRequestUpdate: null,
      rewind: null
    }
  })
  const shown = foldTaskRuns(
    [
      ...rows.slice(0, 4),
      steer,
      said('a is done', 32),
      result('rc', 'c', 33),
      said('fine, thanks', 34),
      said('c is done', 35)
    ],
    stillB
  )

  expect(shape(shown).slice(-4)).toEqual([
    'steer',
    'a is done (completed)',
    'fine, thanks',
    'c is done (completed)'
  ])
})
