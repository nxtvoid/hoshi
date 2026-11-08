import { useEffect } from 'react'
import { useAppStore } from '@/state/app-store'
import { useSidebarStore } from '@/state/sidebar-store'
import { stepThread, visibleOrder } from '@/state/thread-sections'
import { useAttentionThreads, useSidebarSections } from './use-sidebar-sections'

/**
 * Alt+↑/↓ moves through the threads the sidebar shows; with Shift it jumps between the
 * threads that need the user, in every project.
 */
export function useThreadShortcuts() {
  const sections = useSidebarSections()
  const attention = useAttentionThreads()
  const searching = useAppStore((state) => state.search !== '')

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!event.altKey || event.ctrlKey || event.metaKey) return
      if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
      const { selectedThreadId, selectThread } = useAppStore.getState()
      const { collapsed } = useSidebarStore.getState()
      const order = event.shiftKey
        ? attention.map((thread) => thread.id)
        : visibleOrder(
            sections,
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
  }, [sections, attention, searching])
}
