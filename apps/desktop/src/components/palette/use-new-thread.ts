import { useAppStore } from '@/state/app-store'
import { usePaletteStore } from '@/state/palette-store'
import { useSidebarStore } from '@/state/sidebar-store'

/**
 * Opens the project, or the chats for `null`, on an empty composer. The thread only exists
 * once its first message is sent, so asking again doesn't pile up empty threads.
 */
export function useStartThreadIn() {
  const selectProject = useAppStore((state) => state.selectProject)
  const openDraft = useAppStore((state) => state.openDraft)
  const setView = useSidebarStore((state) => state.setView)
  return (projectId: string | null) => {
    setView(projectId === null ? 'chats' : 'project')
    if (projectId === null) openDraft()
    else selectProject(projectId)
  }
}

/**
 * Opens a new thread's composer in the open project, or a new chat's among the chats.
 * Elsewhere, as in the inbox, asks where; returns whether it asks.
 */
export function useNewThread() {
  const startThreadIn = useStartThreadIn()
  const setPaletteOpen = usePaletteStore((state) => state.setOpen)
  return () => {
    const { projects, selectedProjectId } = useAppStore.getState()
    const view = useSidebarStore.getState().view
    if (view === 'project' && selectedProjectId) {
      startThreadIn(selectedProjectId)
    } else if (view === 'chats' || projects.length === 0) {
      startThreadIn(null)
    } else {
      setPaletteOpen(true, 'newThread')
      return true
    }
    return false
  }
}
