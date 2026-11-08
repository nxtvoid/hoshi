import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { Event } from '@/bindings/Event'
import { applyItemEvent } from './orchestration'
import type { ThreadItem } from '@/bindings/ThreadItem'
import type { StreamDelta } from '@/bindings/StreamDelta'
import type { StreamingText } from '@/lib/tool-calls'
import type { TranscriptPage } from '@/bindings/TranscriptPage'

type TranscriptState = {
  /** The thread whose transcript is loaded; only one is kept in memory. */
  threadId: string | null
  loading: boolean
  /** The latest turns are loaded first; older ones as the reader scrolls up. */
  items: ThreadItem[]
  hasOlder: boolean
  loadingOlder: boolean
  /** Text still streaming, by item id, until its final item arrives. */
  streaming: Record<string, StreamingText>
  load: (threadId: string | null) => Promise<void>
  loadOlder: () => Promise<void>
  applyEvent: (event: Event) => void
  applyDelta: (delta: StreamDelta) => void
}

/** Events that arrive while a transcript is loading, replayed on top of it. */
let pending: Event[] = []

const page = (threadId: string, before: string | null) =>
  invoke<TranscriptPage>('get_thread_items', { threadId, before })

export const useTranscriptStore = create<TranscriptState>()((set, get) => ({
  threadId: null,
  loading: false,
  items: [],
  hasOlder: false,
  loadingOlder: false,
  streaming: {},
  load: async (threadId) => {
    pending = []
    set({
      threadId,
      loading: threadId !== null,
      items: [],
      hasOlder: false,
      loadingOlder: false,
      streaming: {}
    })
    if (!threadId) return
    try {
      const loaded = await page(threadId, null)
      if (get().threadId !== threadId) return
      const items = pending.reduce(
        (current, event) => applyItemEvent(threadId, current, event),
        loaded.items
      )
      pending = []
      set({ items, hasOlder: loaded.hasOlder, loading: false })
    } catch (error) {
      console.error('hoshi: failed to load the transcript', error)
      if (get().threadId === threadId) set({ loading: false })
    }
  },
  loadOlder: async () => {
    const { threadId, items, hasOlder, loadingOlder, loading } = get()
    const first = items[0]
    if (!threadId || !first || !hasOlder || loadingOlder || loading) return
    set({ loadingOlder: true })
    try {
      const older = await page(threadId, first.id)
      if (get().threadId !== threadId) return
      set((state) => {
        const known = new Set(state.items.map((item) => item.id))
        const added = older.items.filter((item) => !known.has(item.id))
        return {
          items: [...added, ...state.items],
          hasOlder: older.hasOlder,
          loadingOlder: false
        }
      })
    } catch (error) {
      console.error('hoshi: failed to load older messages', error)
      if (get().threadId === threadId) set({ loadingOlder: false })
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
        : event.type === 'toolCallCompleted'
          ? [event.itemId]
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
        [itemId]: {
          kind,
          text:
            kind === 'toolOutput'
              ? delta
              : (state.streaming[itemId]?.text ?? '') + delta
        }
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
