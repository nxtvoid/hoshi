import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { Event } from '@/bindings/Event'
import { applyItemEvent } from './orchestration'
import type { ThreadItem } from '@/bindings/ThreadItem'
import type { TranscriptPage } from '@/bindings/TranscriptPage'

type SubagentState = {
  /** The transcripts of the subagents being shown, by transcript id; `null` while loading. */
  transcripts: Record<string, ThreadItem[] | null>
  /**
   * The subagent shown in place of the open thread, as the calls that lead to it, outermost
   * first; empty while the thread itself shows.
   */
  viewing: string[]
  /** Shows the subagent a call of the current view started. */
  view: (toolUseId: string) => void
  /** Goes back to what started the subagent shown. */
  leave: () => void
  leaveAll: () => void
  open: (transcriptId: string) => Promise<void>
  close: (transcriptId: string) => void
  applyEvent: (event: Event) => void
}

export const useSubagentStore = create<SubagentState>()((set, get) => ({
  transcripts: {},
  viewing: [],
  view: (toolUseId) =>
    set((state) => ({ viewing: [...state.viewing, toolUseId] })),
  leave: () => set((state) => ({ viewing: state.viewing.slice(0, -1) })),
  leaveAll: () => {
    if (get().viewing.length > 0) set({ viewing: [] })
  },
  open: async (transcriptId) => {
    if (transcriptId in get().transcripts) return
    set((state) => ({
      transcripts: { ...state.transcripts, [transcriptId]: null }
    }))
    try {
      const page = await invoke<TranscriptPage>('get_thread_items', {
        threadId: transcriptId,
        before: null
      })
      if (!(transcriptId in get().transcripts)) return
      set((state) => ({
        transcripts: { ...state.transcripts, [transcriptId]: page.items }
      }))
    } catch (error) {
      console.error('hoshi: failed to load a subagent', error)
    }
  },
  close: (transcriptId) =>
    set((state) => {
      const { [transcriptId]: _, ...rest } = state.transcripts
      return { transcripts: rest }
    }),
  applyEvent: (event) => {
    const { transcripts } = get()
    let changed = false
    const next: Record<string, ThreadItem[] | null> = {}
    for (const [id, items] of Object.entries(transcripts)) {
      const applied = items ? applyItemEvent(id, items, event) : items
      changed ||= applied !== items
      next[id] = applied
    }
    if (changed) set({ transcripts: next })
  }
}))
