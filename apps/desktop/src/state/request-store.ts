import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { Event } from '@/bindings/Event'
import type { ThreadItem } from '@/bindings/ThreadItem'
import {
  isRequest,
  type QuestionProgress,
  type RequestItem
} from '@/lib/requests'

type RequestState = {
  /** How far the user got through each request's questions, by request id. */
  progress: Record<string, QuestionProgress>
  setProgress: (requestId: string, progress: QuestionProgress) => void
  /** What agents of every thread asked and still wait for, oldest first. */
  pending: RequestItem[]
  /** Delegated tasks whose answer hasn't gone back yet, by thread id. */
  pendingTasks: Record<string, true>
  seed: (items: ThreadItem[], pendingTasks: string[]) => void
  applyEvent: (event: Event) => void
  /** Hands a task's answer back to its thread now, as it stands. */
  sendTaskAnswer: (threadId: string) => Promise<void>
  /** Cancels a task: it stops if it runs, and its answer never goes back. */
  cancelTask: (threadId: string) => Promise<void>
  /** Tasks whose card, waiting on the user, is on screen. */
  onScreenTasks: Record<string, true>
  setTaskOnScreen: (threadId: string, onScreen: boolean) => void
}

export const useRequestStore = create<RequestState>()((set, get) => ({
  progress: {},
  setProgress: (requestId, progress) =>
    set((state) => ({
      progress: { ...state.progress, [requestId]: progress }
    })),
  pending: [],
  pendingTasks: {},
  onScreenTasks: {},
  setTaskOnScreen: (threadId, onScreen) =>
    set((state) => {
      if (Boolean(state.onScreenTasks[threadId]) === onScreen) return state
      const { [threadId]: _, ...rest } = state.onScreenTasks
      return { onScreenTasks: onScreen ? { ...rest, [threadId]: true } : rest }
    }),
  seed: (items, pendingTasks) =>
    set({
      pending: items.filter(isRequest),
      pendingTasks: Object.fromEntries(pendingTasks.map((id) => [id, true]))
    }),
  cancelTask: (threadId) =>
    invoke<void>('cancel_task', { threadId }).catch((error) =>
      console.error('hoshi: failed to cancel a task', error)
    ),
  sendTaskAnswer: (threadId) =>
    invoke<void>('send_task_answer', { threadId }).catch((error) =>
      console.error('hoshi: failed to send a task answer', error)
    ),
  applyEvent: (event) => {
    if (event.type === 'threadDelegated') {
      set((state) => ({
        pendingTasks: { ...state.pendingTasks, [event.threadId]: true }
      }))
      return
    }
    const answered =
      event.type === 'taskCancelled' || event.type === 'taskAnswered'
        ? event.threadId
        : event.type === 'itemAdded' &&
            event.item.content.kind === 'userMessage'
          ? event.item.content.taskResult
          : null
    if (answered) {
      const { [answered]: _, ...rest } = get().pendingTasks
      set({ pendingTasks: rest })
      return
    }
    const { pending } = get()
    if (event.type === 'threadDeleted') {
      if (pending.some((item) => item.threadId === event.threadId)) {
        set({
          pending: pending.filter((item) => item.threadId !== event.threadId)
        })
      }
      return
    }
    if (event.type === 'threadRewound') {
      const rewound = (item: RequestItem) =>
        item.threadId.split(':')[0] === event.threadId
      if (pending.some(rewound)) {
        set({ pending: pending.filter((item) => !rewound(item)) })
      }
      return
    }
    if (event.type !== 'itemAdded' || !isRequest(event.item)) return
    const { item } = event
    const rest = pending.filter((request) => request.id !== item.id)
    set({
      pending: item.content.status === 'pending' ? [...rest, item] : rest
    })
  }
}))
