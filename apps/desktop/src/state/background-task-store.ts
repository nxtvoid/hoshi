import { create } from 'zustand'
import type { BackgroundTask } from '@/bindings/BackgroundTask'
import type { ThreadBackgroundTasks } from '@/bindings/ThreadBackgroundTasks'

type ThreadBackground = { tasks: BackgroundTask[]; waiting: boolean }

type BackgroundTaskState = {
  /** The commands each thread's agent runs in the background, by thread id. */
  byThread: Record<string, ThreadBackground>
  /** Task ids the user asked to stop, until they end. */
  stopping: Record<string, true>
  seed: (updates: ThreadBackgroundTasks[]) => void
  receive: (update: ThreadBackgroundTasks) => void
  forget: (threadId: string) => void
  markStopping: (taskId: string) => void
}

function withUpdate(
  byThread: Record<string, ThreadBackground>,
  { threadId, tasks, waiting }: ThreadBackgroundTasks
) {
  const { [threadId]: _, ...rest } = byThread
  return tasks.length === 0 ? rest : { ...rest, [threadId]: { tasks, waiting } }
}

function stillRunning(
  stopping: Record<string, true>,
  byThread: Record<string, ThreadBackground>
) {
  const running = new Set(
    Object.values(byThread).flatMap(({ tasks }) =>
      tasks.map((task) => task.taskId)
    )
  )
  return Object.fromEntries(
    Object.keys(stopping)
      .filter((taskId) => running.has(taskId))
      .map((taskId) => [taskId, true as const])
  )
}

export const useBackgroundTaskStore = create<BackgroundTaskState>()((set) => ({
  byThread: {},
  stopping: {},
  seed: (updates) =>
    set({ byThread: updates.reduce(withUpdate, {}), stopping: {} }),
  receive: (update) =>
    set((state) => {
      const byThread = withUpdate(state.byThread, update)
      return { byThread, stopping: stillRunning(state.stopping, byThread) }
    }),
  forget: (threadId) =>
    set((state) => {
      if (!(threadId in state.byThread)) return state
      const byThread = withUpdate(state.byThread, {
        threadId,
        tasks: [],
        waiting: false
      })
      return { byThread, stopping: stillRunning(state.stopping, byThread) }
    }),
  markStopping: (taskId) =>
    set((state) => ({ stopping: { ...state.stopping, [taskId]: true } }))
}))
