import type { NoteSummary } from '@/bindings/NoteSummary'

/** The notes listed for a project, or for none: its own, and the general ones. */
export type ListedNotes = {
  projectId: string | null
  own: NoteSummary[]
  general: NoteSummary[]
}

const latestFirst = (a: NoteSummary, b: NoteSummary) =>
  b.updatedAt.localeCompare(a.updatedAt) || a.id.localeCompare(b.id)

/**
 * The list once `saved` was written, without reading it again: the note sits where its
 * last edit puts it, among the project's notes or the general ones, and leaves the list
 * when it now belongs to another project.
 */
export function withSavedNote(
  listed: ListedNotes,
  saved: NoteSummary
): ListedNotes {
  const others = (notes: NoteSummary[]) =>
    notes.filter((note) => note.id !== saved.id)
  const place = (notes: NoteSummary[], belongs: boolean) =>
    belongs ? [...others(notes), saved].sort(latestFirst) : others(notes)
  return {
    projectId: listed.projectId,
    own: place(
      listed.own,
      saved.projectId !== null && saved.projectId === listed.projectId
    ),
    general: place(listed.general, saved.projectId === null)
  }
}
