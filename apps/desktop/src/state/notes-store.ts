import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { Note } from '@/bindings/Note'
import { withoutKeys } from '@/lib/records'
import type { NoteSummary } from '@/bindings/NoteSummary'
import type { VaultSecretInput } from '@/bindings/VaultSecretInput'
import type { MessageAttachment } from '@/bindings/MessageAttachment'

/** A note the draft of a message carries, as its chip shows it. */
export type AttachedNote = {
  id: string
  title: string
  /** How many credentials are left out when it is sent. */
  credentials: number
}

type NotesState = {
  /** Goes up when notes change in a way a list can only learn by reading again. */
  revision: number
  /** The note written last, as a list shows it: a list places it without reading again. */
  lastSaved: NoteSummary | null
  /** The notes attached to each unsent message, by draft key. */
  attached: Record<string, AttachedNote[]>
  /** The project whose notes are listed, with the general ones; `null` for those alone. */
  listedProjectId: string | null
  /** The note the main view shows; a new id is a note not written yet. */
  openNoteId: string | null
  /** What the list of notes is searched for. */
  query: string
  /** Goes up when a note is kept from elsewhere in the app, for the rail to say so. */
  captures: number
  touch: () => void
  /**
   * Lists the notes of `projectId`, on `noteId` when given. A note open among another
   * project's notes closes.
   */
  list: (projectId: string | null, noteId?: string) => void
  open: (noteId: string) => void
  close: () => void
  setQuery: (query: string) => void
  captured: () => void
  attach: (draftKey: string, note: AttachedNote) => void
  detach: (draftKey: string, noteId: string) => void
  clearAttached: (draftKey: string) => void
  /** A deleted note leaves every draft. */
  forget: (noteId: string) => void
}

const NO_NOTES: AttachedNote[] = []

export const useNotesStore = create<NotesState>()((set) => ({
  revision: 0,
  lastSaved: null,
  attached: {},
  listedProjectId: null,
  openNoteId: null,
  query: '',
  captures: 0,
  touch: () => set((state) => ({ revision: state.revision + 1 })),
  list: (projectId, noteId) =>
    set((state) => ({
      listedProjectId: projectId,
      openNoteId:
        noteId ??
        (state.listedProjectId === projectId ? state.openNoteId : null)
    })),
  open: (openNoteId) => set({ openNoteId }),
  close: () => set({ openNoteId: null }),
  setQuery: (query) => set({ query }),
  captured: () => set((state) => ({ captures: state.captures + 1 })),
  attach: (draftKey, note) =>
    set((state) => {
      const others = (state.attached[draftKey] ?? NO_NOTES).filter(
        (attached) => attached.id !== note.id
      )
      return { attached: { ...state.attached, [draftKey]: [...others, note] } }
    }),
  detach: (draftKey, noteId) =>
    set((state) => {
      const notes = state.attached[draftKey]
      if (!notes?.some((note) => note.id === noteId)) return state
      const rest = notes.filter((note) => note.id !== noteId)
      return {
        attached:
          rest.length > 0
            ? { ...state.attached, [draftKey]: rest }
            : withoutKeys(state.attached, [draftKey])
      }
    }),
  clearAttached: (draftKey) =>
    set((state) => {
      const attached = withoutKeys(state.attached, [draftKey])
      return attached === state.attached ? state : { attached }
    }),
  forget: (noteId) =>
    set((state) => {
      const attached: Record<string, AttachedNote[]> = {}
      for (const [draftKey, notes] of Object.entries(state.attached)) {
        const rest = notes.filter((note) => note.id !== noteId)
        if (rest.length > 0) attached[draftKey] = rest
      }
      return {
        attached,
        openNoteId: state.openNoteId === noteId ? null : state.openNoteId
      }
    })
}))

/** The notes attached to a draft; stable while none is, so selecting it is cheap. */
export function useAttachedNotes(draftKey: string): AttachedNote[] {
  return useNotesStore((state) => state.attached[draftKey] ?? NO_NOTES)
}

const touch = () => useNotesStore.getState().touch()

/** A project's notes holding `query`, or the general ones for `null`, latest first. */
export function listNotes(projectId: string | null, query: string) {
  return invoke<NoteSummary[]>('list_notes', { projectId, query })
}

/** `null` for a note that doesn't exist, as one not written yet. */
export async function readNote(noteId: string): Promise<Note | null> {
  try {
    return await invoke<Note>('get_note', { noteId })
  } catch {
    return null
  }
}

/**
 * Writes a note's text and where it belongs, adding the note if its id is new. `document`
 * is the same note as its editor shows it; a note written as plain text has none.
 */
export async function saveNote(
  noteId: string,
  projectId: string | null,
  text: string,
  document: string | null = null
) {
  const note = await invoke<Note>('save_note', {
    noteId,
    projectId,
    text,
    document
  })
  useNotesStore.setState({
    lastSaved: {
      id: note.id,
      projectId: note.projectId,
      title: note.title,
      preview: note.preview,
      updatedAt: note.updatedAt,
      credentials: note.credentials.length
    }
  })
  return note
}

export async function deleteNote(noteId: string) {
  await invoke('delete_note', { noteId })
  useNotesStore.getState().forget(noteId)
  touch()
}

/** The user says the note's `index`th credential is none. */
export async function ignoreNoteCredential(noteId: string, index: number) {
  const note = await invoke<Note>('ignore_note_credential', { noteId, index })
  touch()
  return note
}

/** Moves the note's `index`th credential into the vault, described by `secret`. */
export async function moveNoteCredentialToVault(
  noteId: string,
  index: number,
  secret: VaultSecretInput
) {
  const note = await invoke<Note>('move_note_credential_to_vault', {
    noteId,
    index,
    secret
  })
  touch()
  return note
}

/**
 * What a message carries for each attached note, read now and without its credentials. A
 * note deleted since it was attached is left out.
 */
export async function noteAttachments(
  notes: readonly AttachedNote[]
): Promise<MessageAttachment[]> {
  const read = await Promise.all(
    notes.map((note) =>
      invoke<MessageAttachment>('get_note_attachment', {
        noteId: note.id
      }).catch(() => null)
    )
  )
  return read.filter((attachment) => attachment !== null)
}
