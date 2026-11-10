import { expect, test } from 'bun:test'
import type { Thread } from '../../src/bindings/Thread'
import type { ThreadItem } from '../../src/bindings/ThreadItem'
import {
  plainPreview,
  runningTasks,
  TASK_PREVIEW_CHARS,
  taskPreview,
  taskState,
  taskTime,
  threadAgents
} from '../../src/lib/agents'

const thread = (id: string, changes: Partial<Thread>): Thread =>
  ({
    id,
    projectId: 'p',
    title: id,
    status: 'idle',
    parentThreadId: null,
    createdAt: '2026-11-08T10:00:00Z',
    ...changes
  }) as Thread

const agentCall = (
  id: string,
  createdAt: string,
  changes: { output: string | null; interrupted?: boolean }
): ThreadItem => ({
  id,
  threadId: 't',
  createdAt,
  content: {
    kind: 'toolCall',
    name: 'Agent',
    input: JSON.stringify({
      description: `look ${id}`,
      subagent_type: 'Explore'
    }),
    output: changes.output,
    isError: false,
    detail: null,
    outputOmitted: false,
    interrupted: changes.interrupted ?? false
  }
})

test('lists the tasks a thread delegated and its subagents, newest first', () => {
  const threads = [
    thread('task', {
      parentThreadId: 't',
      title: 'Count files',
      status: 'waitingForInput',
      createdAt: '2026-11-08T10:05:00Z'
    }),
    thread('other', { parentThreadId: 'x' })
  ]
  const items = [
    agentCall('a1', '2026-11-08T10:01:00Z', { output: 'done' }),
    agentCall('a2', '2026-11-08T10:09:00Z', { output: null }),
    agentCall('a3', '2026-11-08T10:03:00Z', { output: null, interrupted: true })
  ]

  const agents = threadAgents('t', threads, items)

  expect(
    agents.map(({ kind, id, title, state }) => [kind, id, title, state])
  ).toEqual([
    ['subagent', 'a2', 'look a2', 'running'],
    ['task', 'task', 'Count files', 'waiting'],
    ['subagent', 'a3', 'look a3', 'stopped'],
    ['subagent', 'a1', 'look a1', 'done']
  ])
})

test('a task waiting for changes to its plan counts as waiting', () => {
  const planning = thread('plan', {
    parentThreadId: 't',
    planMode: true,
    status: 'idle'
  })

  expect(threadAgents('t', [planning], [], { plan: true })[0]?.state).toBe(
    'waiting'
  )
  expect(threadAgents('t', [planning], [])[0]?.state).toBe('done')
})

test('a cancelled task counts as stopped', () => {
  const cancelled = thread('gone', {
    parentThreadId: 't',
    taskCancelled: true,
    status: 'idle'
  })

  expect(threadAgents('t', [cancelled], [])[0]?.state).toBe('stopped')
})

test('a task reads as running, waiting, finished, stopped or failed', () => {
  const task = (changes: Partial<Thread>) =>
    thread('task', { parentThreadId: 't', ...changes })

  expect(taskState(task({ status: 'working' }), {})).toBe('running')
  expect(taskState(task({ status: 'waitingForInput' }), {})).toBe('waiting')
  expect(taskState(task({ status: 'idle' }), {})).toBe('done')
  expect(taskState(task({ status: 'error' }), {})).toBe('failed')
  expect(
    taskState(task({ status: 'idle', taskCancelled: true }), { task: true })
  ).toBe('stopped')
  expect(taskState(task({ status: 'working', taskCancelled: true }), {})).toBe(
    'running'
  )
})

test('a running task counts its time from its start, an ended one keeps its end', () => {
  const task = thread('task', {
    parentThreadId: 't',
    createdAt: '2026-11-08T10:00:00Z',
    updatedAt: '2026-11-08T10:01:08Z'
  })
  const now = Date.parse('2026-11-08T10:00:19Z')

  expect(taskTime(task, 'running', now)).toEqual({
    kind: 'elapsed',
    milliseconds: 19_000
  })
  expect(taskTime(task, 'waiting', now).kind).toBe('elapsed')
  for (const state of ['done', 'stopped', 'failed'] as const) {
    expect(taskTime(task, state, now)).toEqual({
      kind: 'ended',
      at: '2026-11-08T10:01:08Z'
    })
  }
})

test('a thread waits on the tasks that still run and owe it an answer', () => {
  const threads = [
    thread('later', {
      parentThreadId: 't',
      status: 'working',
      createdAt: '2026-11-08T10:05:00Z'
    }),
    thread('first', { parentThreadId: 't', status: 'working' }),
    thread('done', { parentThreadId: 't', status: 'idle' }),
    thread('asks', { parentThreadId: 't', status: 'waitingForInput' }),
    thread('cancelled', {
      parentThreadId: 't',
      status: 'working',
      taskCancelled: true
    }),
    thread('answered', { parentThreadId: 't', status: 'working' }),
    thread('elsewhere', { parentThreadId: 'x', status: 'working' })
  ]
  const pending = Object.fromEntries(
    ['later', 'first', 'done', 'asks', 'cancelled', 'elsewhere'].map((id) => [
      id,
      true as const
    ])
  )

  expect(runningTasks('t', threads, pending).map(({ id }) => id)).toEqual([
    'first',
    'later'
  ])
})

test('a task preview is one short line, like hoshi-core makes it', () => {
  expect(taskPreview('  Found it.\n\n- one\n- two  ')).toBe(
    'Found it. - one - two'
  )
  const preview = taskPreview('word '.repeat(100))
  expect(Array.from(preview)).toHaveLength(TASK_PREVIEW_CHARS)
  expect(preview.endsWith('word…')).toBe(true)
})

test('a preview reads as plain text', () => {
  expect(plainPreview('**Done**: see [the PR](https://x.y/1) and `a.ts`')).toBe(
    'Done: see the PR and a.ts'
  )
  expect(plainPreview('## Found threads.watching_pull_request')).toBe(
    'Found threads.watching_pull_request'
  )
})
