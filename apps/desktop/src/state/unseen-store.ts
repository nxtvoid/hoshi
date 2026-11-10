import { useEffect } from 'react'
import { create } from 'zustand'
import { withoutKeys } from '@/lib/records'
import { persist } from 'zustand/middleware'
import { useAppStore } from './app-store'
import { useSidebarStore } from './sidebar-store'
import { finishedTurns } from './thread-signals'
import { unlistedTaskIds } from './thread-sections'
import { removedThreadIds } from './orchestration'

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
  forget: (threadIds: string[]) => void
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
        }),
      forget: (threadIds) =>
        set((state) => {
          const unseen = withoutKeys(state.unseen, threadIds)
          return unseen === state.unseen ? state : { unseen }
        })
    }),
    {
      name: 'hoshi-unseen',
      partialize: (state) => ({ unseen: state.unseen })
    }
  )
)

/**
 * The open thread shows: the window has focus, and neither an overlay, such as settings,
 * nor the notes cover it.
 */
function onScreen() {
  return (
    document.hasFocus() &&
    window.location.pathname === '/' &&
    !useSidebarStore.getState().notesOpen
  )
}

/**
 * Marks a turn that ends while its thread isn't on screen, and unmarks a thread once the
 * user opens it, or comes back to the window with it open. Returns how to stop.
 */
export function trackUnseen(): () => void {
  const { mark, see, forget } = useUnseenStore.getState()
  const seeSelected = () => {
    const { selectedThreadId } = useAppStore.getState()
    if (selectedThreadId && onScreen()) see(selectedThreadId)
  }
  const unsubscribe = useAppStore.subscribe((state, previous) => {
    if (state.threads !== previous.threads) {
      const watched = onScreen() ? state.selectedThreadId : null
      const unlisted = new Set(unlistedTaskIds(state.threads))
      const finished = finishedTurns(previous.threads, state.threads).filter(
        (id) => id !== watched && !unlisted.has(id)
      )
      if (finished.length > 0) mark(finished)
      const { unseen } = useUnseenStore.getState()
      const gone = [
        ...removedThreadIds(previous.threads, state.threads),
        ...[...unlisted].filter((id) => unseen[id])
      ]
      if (gone.length > 0) forget(gone)
    }
    if (state.selectedThreadId !== previous.selectedThreadId) seeSelected()
  })
  const unsubscribeNotes = useSidebarStore.subscribe((state, previous) => {
    if (previous.notesOpen && !state.notesOpen) seeSelected()
  })
  window.addEventListener('focus', seeSelected)
  return () => {
    unsubscribe()
    unsubscribeNotes()
    window.removeEventListener('focus', seeSelected)
  }
}

export function useUnseenTracking() {
  useEffect(() => trackUnseen(), [])
}
