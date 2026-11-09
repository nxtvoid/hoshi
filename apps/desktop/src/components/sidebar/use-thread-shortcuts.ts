import { useEffect } from 'react'
import { useAppStore } from '@/state/app-store'
import { useSidebarStore } from '@/state/sidebar-store'
import { stepThread, visibleOrder } from '@/state/thread-sections'
import {
  currentAttentionThreads,
  currentSidebarSections
} from './use-sidebar-sections'

/**
 * Alt+↑/↓ moves through the threads the sidebar shows; with Shift it jumps between the
 * threads that need the user, in every project.
 */
export function useThreadShortcuts() {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!event.altKey || event.ctrlKey || event.metaKey) return
      if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
      const { selectedThreadId, selectThread, search } = useAppStore.getState()
      const { collapsed } = useSidebarStore.getState()
      const searching = search !== ''
      const order = event.shiftKey
        ? currentAttentionThreads().map((thread) => thread.id)
        : visibleOrder(
            currentSidebarSections(),
            (section) => searching || collapsed[section] !== true,
            selectedThreadId
          )
      const next = stepThread(
        order,
        selectedThreadId,
        event.key === 'ArrowDown' ? 1 : -1
      )
      if (!next) return
      event.preventDefault()
      selectThread(next)
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])
}
