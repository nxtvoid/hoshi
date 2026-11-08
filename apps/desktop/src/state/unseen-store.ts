import { useEffect } from 'react'
import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { useAppStore } from './app-store'
import { finishedTurns } from './thread-signals'

type UnseenState = {
  /** Threads whose last turn finished while the user looked elsewhere. */
  unseen: Record<string, true>
  /**
   * The open thread, when it was unseen as it opened. The inbox keeps it where it was until
   * another one opens, so the list doesn't move under the cursor.
   */
  held: string | null
  mark: (threadIds: string[]) => void
  see: (threadId: string) => void
}

export const useUnseenStore = create<UnseenState>()(
  persist(
    (set) => ({
      unseen: {},
      held: null,
      mark: (threadIds) =>
        set((state) => ({
          unseen: {
            ...state.unseen,
            ...Object.fromEntries(threadIds.map((id) => [id, true as const]))
          }
        })),
      see: (threadId) =>
        set((state) => {
          const held =
            state.unseen[threadId] || state.held === threadId ? threadId : null
          if (!state.unseen[threadId]) return { held }
          const { [threadId]: _, ...unseen } = state.unseen
          return { unseen, held }
        })
    }),
    {
      name: 'hoshi-unseen',
      partialize: (state) => ({ unseen: state.unseen })
    }
  )
)

/** The open thread shows: the window has focus and no overlay, such as settings, covers it. */
function onScreen() {
  return document.hasFocus() && window.location.pathname === '/'
}

/**
 * Marks a turn that ends while its thread isn't on screen, and unmarks a thread once the
 * user opens it, or comes back to the window with it open.
 */
export function useUnseenTracking() {
  useEffect(() => {
    const { mark, see } = useUnseenStore.getState()
    const seeSelected = () => {
      const { selectedThreadId } = useAppStore.getState()
      if (selectedThreadId && onScreen()) see(selectedThreadId)
    }
    const unsubscribe = useAppStore.subscribe((state, previous) => {
      if (state.threads !== previous.threads) {
        const watched = onScreen() ? state.selectedThreadId : null
        const finished = finishedTurns(previous.threads, state.threads).filter(
          (id) => id !== watched
        )
        if (finished.length > 0) mark(finished)
      }
      if (state.selectedThreadId !== previous.selectedThreadId) seeSelected()
    })
    window.addEventListener('focus', seeSelected)
    return () => {
      unsubscribe()
      window.removeEventListener('focus', seeSelected)
    }
  }, [])
}
