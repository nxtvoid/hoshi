import { create } from 'zustand'
import { persist } from 'zustand/middleware'

type CleanupState = {
  /** Threads whose clean-up offer the user turned down. */
  dismissed: Record<string, true>
  dismiss: (threadId: string) => void
}

export const useCleanupStore = create<CleanupState>()(
  persist(
    (set) => ({
      dismissed: {},
      dismiss: (threadId) =>
        set((state) => ({
          dismissed: { ...state.dismissed, [threadId]: true }
        }))
    }),
    { name: 'hoshi-cleanup' }
  )
)
