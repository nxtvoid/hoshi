import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { Event } from '@/bindings/Event'
import { applyItemEvent } from './orchestration'
import type { ThreadItem } from '@/bindings/ThreadItem'
import type { StreamDelta } from '@/bindings/StreamDelta'
import type { StreamingText } from '@/lib/tool-calls'

type TranscriptState = {
  /** The thread whose transcript is loaded; only one is kept in memory. */
  threadId: string | null
  loading: boolean
  items: ThreadItem[]
  /** Text still streaming, by item id, until its final item arrives. */
  streaming: Record<string, StreamingText>
  load: (threadId: string | null) => Promise<void>
  applyEvent: (event: Event) => void
  applyDelta: (delta: StreamDelta) => void
}

/** Events that arrive while a transcript is loading, replayed on top of it. */
let pending: Event[] = []

export const useTranscriptStore = create<TranscriptState>()((set, get) => ({
  threadId: null,
  loading: false,
  items: [],
  streaming: {},
  load: async (threadId) => {
    pending = []
    set({ threadId, loading: threadId !== null, items: [], streaming: {} })
    if (!threadId) return
    try {
      const loaded = await invoke<ThreadItem[]>('get_thread_items', {
        threadId
      })
      if (get().threadId !== threadId) return
      const items = pending.reduce(
        (current, event) => applyItemEvent(threadId, current, event),
        loaded
      )
      pending = []
      set({ items, loading: false })
    } catch (error) {
      console.error('hoshi: failed to load the transcript', error)
      if (get().threadId === threadId) set({ loading: false })
    }
  },
  applyEvent: (event) => {
    const { threadId, loading, items, streaming } = get()
    if (!threadId) return
    if (loading) {
      pending.push(event)
      return
    }
    const next = applyItemEvent(threadId, items, event)
    if (next === items && event.type !== 'turnCompleted') return
    const finished =
      event.type === 'itemAdded'
        ? [event.item.id]
        : event.type === 'turnCompleted' && event.threadId === threadId
          ? Object.keys(streaming)
          : []
    set({ items: next, streaming: omit(streaming, finished) })
  },
  applyDelta: ({ threadId, itemId, kind, delta }) => {
    if (threadId !== get().threadId) return
    set((state) => ({
      streaming: {
        ...state.streaming,
        [itemId]: { kind, text: (state.streaming[itemId]?.text ?? '') + delta }
      }
    }))
  }
}))

function omit<T>(record: Record<string, T>, keys: string[]) {
  if (!keys.some((key) => key in record)) return record
  const next = { ...record }
  for (const key of keys) delete next[key]
  return next
}
