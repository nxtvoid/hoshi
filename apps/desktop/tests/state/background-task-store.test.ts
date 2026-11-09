import { expect, test } from 'bun:test'
import type { BackgroundTask } from '../../src/bindings/BackgroundTask'
import { useBackgroundTaskStore } from '../../src/state/background-task-store'

const task = (taskId: string): BackgroundTask => ({
  taskId,
  toolCallId: null,
  description: 'Start the dev server',
  startedAt: 1
})

test('keeps the background commands of each running turn until none is left', () => {
  const store = useBackgroundTaskStore.getState()
  store.seed([{ threadId: 't1', tasks: [task('b1')], waiting: true }])
  store.receive({ threadId: 't2', tasks: [task('b2')], waiting: false })
  expect(Object.keys(useBackgroundTaskStore.getState().byThread)).toEqual([
    't1',
    't2'
  ])

  store.receive({ threadId: 't1', tasks: [], waiting: true })
  store.forget('t2')
  expect(useBackgroundTaskStore.getState().byThread).toEqual({})
})

test('a command asked to stop shows so until it ends', () => {
  const store = useBackgroundTaskStore.getState()
  store.seed([
    { threadId: 't1', tasks: [task('b1'), task('b2')], waiting: true }
  ])
  store.markStopping('b1')
  store.receive({ threadId: 't1', tasks: [task('b1')], waiting: true })
  expect(useBackgroundTaskStore.getState().stopping).toEqual({ b1: true })

  store.receive({ threadId: 't1', tasks: [], waiting: true })
  expect(useBackgroundTaskStore.getState().stopping).toEqual({})
})
