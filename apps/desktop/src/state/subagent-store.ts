import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { Event } from '@/bindings/Event'
import { applyItemEvent } from './orchestration'
import type { ThreadItem } from '@/bindings/ThreadItem'
import type { TranscriptPage } from '@/bindings/TranscriptPage'

type SubagentState = {
  /** The transcripts of the subagents whose card is open, by transcript id; `null` while loading. */
  transcripts: Record<string, ThreadItem[] | null>
  open: (transcriptId: string) => Promise<void>
  close: (transcriptId: string) => void
  applyEvent: (event: Event) => void
}

export const useSubagentStore = create<SubagentState>()((set, get) => ({
  transcripts: {},
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
