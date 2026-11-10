import { expect, test } from 'bun:test'
import type { RequestStatus } from '../../src/bindings/RequestStatus'
import type { ThreadItem } from '../../src/bindings/ThreadItem'
import { useRequestStore } from '../../src/state/request-store'

const approval = (
  id: string,
  threadId: string,
  status: RequestStatus
): ThreadItem => ({
  id,
  threadId,
  createdAt: '2026-11-08T10:00:00Z',
  content: {
    kind: 'approval',
    toolCallId: null,
    toolName: 'Bash',
    input: '{}',
    reason: null,
    canAcceptForSession: true,
    status,
    decision: null,
    feedback: null
  }
})

test('keeps what every thread still waits for', () => {
  const store = useRequestStore.getState()
  store.seed([approval('a', 'child', 'pending')], [])
  store.applyEvent({
    type: 'itemAdded',
    item: approval('b', 'other', 'pending')
  })
  store.applyEvent({
    type: 'itemAdded',
    item: approval('a', 'child', 'resolved')
  })

  expect(useRequestStore.getState().pending.map((item) => item.id)).toEqual([
    'b'
  ])

  store.applyEvent({ type: 'threadDeleted', threadId: 'other' })
  expect(useRequestStore.getState().pending).toEqual([])
})

test('keeps the tasks that still owe their thread an answer', () => {
  const store = useRequestStore.getState()
  store.seed([], ['old'])
  store.applyEvent({
    type: 'threadDelegated',
    threadId: 'task',
    parentThreadId: 't'
  })
  store.applyEvent({
    type: 'itemAdded',
    item: {
      id: 'r',
      threadId: 't',
      createdAt: '2026-11-08T10:00:00Z',
      content: {
        kind: 'userMessage',
        text: 'Delegated task old',
        attachments: [],
        steer: false,
        taskResult: 'old'
      }
    }
  })

  expect(useRequestStore.getState().pendingTasks).toEqual({ task: true })

  store.applyEvent({ type: 'taskCancelled', threadId: 'task' })
  expect(useRequestStore.getState().pendingTasks).toEqual({})

  store.applyEvent({ type: 'taskReopened', threadId: 'old' })
  expect(useRequestStore.getState().pendingTasks).toEqual({ old: true })
})

test('drops the requests of a thread that went back before them', () => {
  const store = useRequestStore.getState()
  store.seed(
    [
      approval('a', 't', 'pending'),
      approval('b', 't:agent', 'pending'),
      approval('c', 'other', 'pending')
    ],
    []
  )

  store.applyEvent({
    type: 'threadRewound',
    threadId: 't',
    messageId: 'm1',
    resumeAt: null
  })

  expect(useRequestStore.getState().pending.map((item) => item.id)).toEqual([
    'c'
  ])
})

test('keeps the latest message of each delegated task', () => {
  const store = useRequestStore.getState()
  store.seed([], ['task'], [{ threadId: 'old', text: 'Earlier answer' }])
  const say = (threadId: string, id: string, text: string) =>
    store.applyEvent({
      type: 'itemAdded',
      item: {
        id,
        threadId,
        createdAt: '2026-11-08T10:00:00Z',
        content: { kind: 'assistantMessage', text }
      }
    })

  say('task', 'm1', 'Looking\n  around')
  say('old', 'm2', 'More')
  say('parent', 'm3', 'Not a task')

  expect(useRequestStore.getState().taskPreviews).toEqual({
    task: 'Looking around',
    old: 'More'
  })

  store.applyEvent({ type: 'threadDeleted', threadId: 'old' })
  expect(useRequestStore.getState().taskPreviews).toEqual({
    task: 'Looking around'
  })
})
