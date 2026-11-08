import { create } from 'zustand'
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
  seed: (items: ThreadItem[]) => void
  applyEvent: (event: Event) => void
}

export const useRequestStore = create<RequestState>()((set, get) => ({
  progress: {},
  setProgress: (requestId, progress) =>
    set((state) => ({
      progress: { ...state.progress, [requestId]: progress }
    })),
  pending: [],
  seed: (items) => set({ pending: items.filter(isRequest) }),
  applyEvent: (event) => {
    const { pending } = get()
    if (event.type === 'threadDeleted') {
      if (pending.some((item) => item.threadId === event.threadId)) {
        set({
          pending: pending.filter((item) => item.threadId !== event.threadId)
        })
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
