import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { Event } from '@/bindings/Event'
import { applyItemEvent } from './orchestration'
import type { ThreadItem } from '@/bindings/ThreadItem'
import type { StreamDelta } from '@/bindings/StreamDelta'
import type { StreamingText } from '@/lib/tool-calls'
import { frameScheduler } from '@/lib/frame-scheduler'
import type { TranscriptPage } from '@/bindings/TranscriptPage'
import { applyDeltas, bufferDelta, type LiveText } from '@/lib/stream-deltas'

type TranscriptState = {
  /** The thread whose transcript is shown. */
  threadId: string | null
  /** Its transcript is on its way; `items` may still be the previous thread's meanwhile. */
  loading: boolean
  /** The latest turns are loaded first; older ones as the reader scrolls up. */
  items: ThreadItem[]
  hasOlder: boolean
  loadingOlder: boolean
  /** Text still streaming, by item id, until its final item arrives. */
  streaming: Record<string, StreamingText>
  /** Running tools' output so far, by tool call id, until the call completes. */
  toolOutput: Record<string, string>
  load: (threadId: string | null) => Promise<void>
  loadOlder: () => Promise<void>
  applyEvent: (event: Event) => void
  applyDelta: (delta: StreamDelta) => void
}

/** Events that arrive while a transcript is loading, replayed on top of it. */
let pending: Event[] = []

/** Deltas not shown yet, applied together once a frame. */
const deltas = new Map<string, StreamingText>()

/** Streaming text may take this much of the thread's time; the rest stays for the user. */
const STREAMING_SHARE = 0.25

const flush = frameScheduler(
  () => {
    const state = useTranscriptStore.getState()
    const live = takeDeltas(state)
    if (live !== state) useTranscriptStore.setState(live)
  },
  { share: STREAMING_SHARE }
)

function takeDeltas(live: LiveText): LiveText {
  flush.cancel()
  if (deltas.size === 0) return live
  const next = applyDeltas(live, deltas)
  deltas.clear()
  return next
}

const page = (threadId: string, before: string | null) =>
  invoke<TranscriptPage>('get_thread_items', { threadId, before })

type Kept = { items: ThreadItem[]; hasOlder: boolean }

/** How many transcripts besides the one shown stay in memory, to open again at once. */
const KEPT_TRANSCRIPTS = 8
/** How long the previous thread stays on screen while the next one's transcript loads. */
const BLANK_AFTER_MS = 400

/**
 * Transcripts opened lately, the latest last, kept up to date with the events that reach
 * the app, so going back to one shows it at once.
 */
const kept = new Map<string, Kept>()

function keep(threadId: string, transcript: Kept) {
  kept.delete(threadId)
  kept.set(threadId, transcript)
  while (kept.size > KEPT_TRANSCRIPTS) {
    const [oldest] = kept.keys()
    if (oldest === undefined) break
    kept.delete(oldest)
  }
}

/** Applies an event to the transcripts kept in memory that aren't on screen. */
function applyToKept(event: Event, shown: string | null) {
  if (event.type === 'threadDeleted') kept.delete(event.threadId)
  for (const [threadId, transcript] of kept) {
    if (threadId === shown) continue
    const items = applyItemEvent(threadId, transcript.items, event)
    if (items !== transcript.items) kept.set(threadId, { ...transcript, items })
  }
}

let blankTimer: ReturnType<typeof setTimeout> | undefined

export const useTranscriptStore = create<TranscriptState>()((set, get) => ({
  threadId: null,
  loading: false,
  items: [],
  hasOlder: false,
  loadingOlder: false,
  streaming: {},
  toolOutput: {},
  load: async (threadId) => {
    const previous = get()
    if (previous.threadId && !previous.loading) {
      keep(previous.threadId, {
        items: previous.items,
        hasOlder: previous.hasOlder
      })
    }
    pending = []
    flush.cancel()
    deltas.clear()
    clearTimeout(blankTimer)
    const known = threadId ? kept.get(threadId) : undefined
    if (!threadId || known) {
      set({
        threadId,
        loading: false,
        items: known?.items ?? [],
        hasOlder: known?.hasOlder ?? false,
        loadingOlder: false,
        streaming: {},
        toolOutput: {}
      })
      return
    }
    set({
      threadId,
      loading: true,
      loadingOlder: false,
      streaming: {},
      toolOutput: {}
    })
    blankTimer = setTimeout(() => {
      if (get().threadId === threadId && get().loading) {
        set({ items: [], hasOlder: false })
      }
    }, BLANK_AFTER_MS)
    try {
      const loaded = await page(threadId, null)
      if (get().threadId !== threadId) return
      const items = pending.reduce(
        (current, event) => applyItemEvent(threadId, current, event),
        loaded.items
      )
      pending = []
      clearTimeout(blankTimer)
      set({ items, hasOlder: loaded.hasOlder, loading: false })
    } catch (error) {
      console.error('hoshi: failed to load the transcript', error)
      if (get().threadId === threadId) {
        clearTimeout(blankTimer)
        set({ items: [], hasOlder: false, loading: false })
      }
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
    const state = get()
    const { threadId, loading, items } = state
    applyToKept(event, threadId)
    if (!threadId) return
    if (loading) {
      pending.push(event)
      return
    }
    const { streaming, toolOutput } = takeDeltas(state)
    const flushed =
      streaming !== state.streaming || toolOutput !== state.toolOutput
    const next = applyItemEvent(threadId, items, event)
    if (next === items && event.type !== 'turnCompleted') {
      if (flushed) set({ streaming, toolOutput })
      return
    }
    // A steer cuts short whatever the agent was still writing.
    const steered =
      event.type === 'itemAdded' &&
      event.item.content.kind === 'userMessage' &&
      event.item.content.steer
    const all =
      steered || (event.type === 'turnCompleted' && event.threadId === threadId)
    const finished =
      event.type === 'itemAdded'
        ? [event.item.id]
        : event.type === 'toolCallCompleted'
          ? [event.itemId]
          : []
    set({
      items: next,
      streaming: all ? omitAll(streaming) : omit(streaming, finished),
      toolOutput: all ? omitAll(toolOutput) : omit(toolOutput, finished)
    })
  },
  applyDelta: (delta) => {
    if (delta.threadId !== get().threadId) return
    bufferDelta(deltas, delta)
    flush.schedule()
  }
}))

const indexes = new WeakMap<ThreadItem[], Map<string, ThreadItem>>()

/** The item with this id; the index is built once per list, so selectors stay cheap. */
export function itemById(items: ThreadItem[], id: string) {
  let index = indexes.get(items)
  if (!index) {
    index = new Map(items.map((item) => [item.id, item]))
    indexes.set(items, index)
  }
  return index.get(id)
}

function omit<T>(record: Record<string, T>, keys: string[]) {
  if (!keys.some((key) => key in record)) return record
  const next = { ...record }
  for (const key of keys) delete next[key]
  return next
}

function omitAll<T>(record: Record<string, T>): Record<string, T> {
  return Object.keys(record).length === 0 ? record : {}
}
