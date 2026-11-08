import { expect, test } from 'bun:test'
import type { Thread } from '../../src/bindings/Thread'
import type { ThreadItem } from '../../src/bindings/ThreadItem'
import { threadAgents } from '../../src/lib/agents'

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
