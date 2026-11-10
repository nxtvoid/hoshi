import { useAppStore } from './app-store'
import { scopeProjectId } from './scope'
import type { Note } from '@/bindings/Note'
import { useNotesStore } from './notes-store'
import { useSidebarStore } from './sidebar-store'

/** Shows the notes of the project the main view works in, in the sidebar and the main view. */
export function showNotes() {
  const projectId = scopeProjectId(
    useAppStore.getState(),
    useSidebarStore.getState().view
  )
  useNotesStore.getState().list(projectId)
  useSidebarStore.getState().setNotesOpen(true)
}

/** Shows a note among the notes of the project it belongs to, wherever the user was. */
export function showNote(note: Note) {
  const known = useAppStore
    .getState()
    .projects.some((project) => project.id === note.projectId)
  useNotesStore.getState().list(known ? note.projectId : null, note.id)
  useSidebarStore.getState().setNotesOpen(true)
}

/**
 * The project whose notes are listed, with the general ones; `null` for those alone, as
 * once that project is gone.
 */
export function useNotesProjectId() {
  const listed = useNotesStore((state) => state.listedProjectId)
  return useAppStore((state) =>
    listed !== null && state.projects.some((project) => project.id === listed)
      ? listed
      : null
  )
}
