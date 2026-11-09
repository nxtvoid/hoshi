import { create } from 'zustand'
import { withoutKeys } from '@/lib/records'
import { persist } from 'zustand/middleware'

type CleanupState = {
  /** Threads whose clean-up offer the user turned down. */
  dismissed: Record<string, true>
  dismiss: (threadId: string) => void
  forget: (threadIds: string[]) => void
}

export const useCleanupStore = create<CleanupState>()(
  persist(
    (set) => ({
      dismissed: {},
      dismiss: (threadId) =>
        set((state) => ({
          dismissed: { ...state.dismissed, [threadId]: true }
        })),
      forget: (threadIds) =>
        set((state) => {
          const dismissed = withoutKeys(state.dismissed, threadIds)
          return dismissed === state.dismissed ? state : { dismissed }
        })
    }),
    { name: 'hoshi-cleanup' }
  )
)
