import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { withoutKeys } from '@/lib/records'
import { invoke } from '@tauri-apps/api/core'
import type { Editor } from '@/bindings/Editor'
import { debouncedStorage, flushOnLeave } from './debounced-storage'
import type { ChangesSummary } from '@/bindings/ChangesSummary'

type DetailsState = {
  /** Editors on this machine, as last found; `null` until the first answer. */
  editors: Editor[] | null
  /** Uncommitted changes of each thread, as last read, shown until a fresh read arrives. */
  changes: Record<string, ChangesSummary>
  loadEditors: () => Promise<void>
  loadChanges: (threadId: string) => Promise<void>
  forget: (threadIds: string[]) => void
}

const sameChanges = (a: ChangesSummary | undefined, b: ChangesSummary) =>
  a?.files === b.files &&
  a.additions === b.additions &&
  a.deletions === b.deletions

const storage =
  debouncedStorage<Pick<DetailsState, 'editors' | 'changes'>>(1000)
flushOnLeave(storage.flush)

/** What the thread details card shows beyond the app store, cached so it opens at once. */
export const useDetailsStore = create<DetailsState>()(
  persist(
    (set, get) => ({
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
          set((state) =>
            sameChanges(state.changes[threadId], summary)
              ? state
              : { changes: { ...state.changes, [threadId]: summary } }
          )
        } catch {
          get().forget([threadId])
        }
      },
      forget: (threadIds) =>
        set((state) => {
          const changes = withoutKeys(state.changes, threadIds)
          return changes === state.changes ? state : { changes }
        })
    }),
    {
      name: 'hoshi-details',
      storage,
      partialize: ({ editors, changes }) => ({ editors, changes })
    }
  )
)
