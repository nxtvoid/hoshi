import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { invoke } from '@tauri-apps/api/core'
import type { Editor } from '@/bindings/Editor'
import type { ChangesSummary } from '@/bindings/ChangesSummary'

type DetailsState = {
  /** Editors on this machine, as last found; `null` until the first answer. */
  editors: Editor[] | null
  /** Uncommitted changes of each thread, as last read, shown until a fresh read arrives. */
  changes: Record<string, ChangesSummary>
  loadEditors: () => Promise<void>
  loadChanges: (threadId: string) => Promise<void>
}

/** What the thread details card shows beyond the app store, cached so it opens at once. */
export const useDetailsStore = create<DetailsState>()(
  persist(
    (set) => ({
      editors: null,
      changes: {},
      loadEditors: async () => {
        try {
          set({ editors: await invoke<Editor[]>('get_editors') })
        } catch (error) {
          console.error('hoshi: failed to find editors', error)
        }
      },
      loadChanges: async (threadId) => {
        try {
          const summary = await invoke<ChangesSummary>('get_thread_changes', {
            threadId
          })
          set((state) => ({
            changes: { ...state.changes, [threadId]: summary }
          }))
        } catch {
          set((state) => {
            const { [threadId]: _, ...changes } = state.changes
            return { changes }
          })
        }
      }
    }),
    { name: 'hoshi-details' }
  )
)
