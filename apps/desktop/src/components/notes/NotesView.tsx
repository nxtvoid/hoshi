import { useEffect } from 'react'
import { NoteEditor } from './NoteEditor'
import { m } from '@/paraglide/messages'
import { NotesHeader } from './NotesHeader'
import { useAppStore } from '@/state/app-store'
import { LogoMark } from '@/components/brand/Logo'
import { useNotesStore } from '@/state/notes-store'
import { useNotesProjectId } from '@/state/notes-hub'
import { Button } from '@hoshi/ui/components/button'
import { useSidebarStore } from '@/state/sidebar-store'
import { QUICK_NOTE_SHORTCUT } from './quick-note-shortcut'

/**
 * The user's notes in the main view, in place of the open thread: the note the sidebar's
 * list opened, in its editor. A note moved to another project takes the list there, so
 * what is listed is always where the open note belongs; opening a thread from anywhere
 * leaves the notes.
 */
export function NotesView() {
  const projectId = useNotesProjectId()
  const noteId = useNotesStore((state) => state.openNoteId)
  const open = useNotesStore((state) => state.open)
  const close = useNotesStore((state) => state.close)
  const list = useNotesStore((state) => state.list)

  useEffect(
    () =>
      useAppStore.subscribe((state, previous) => {
        if (
          state.selectedThreadId &&
          state.selectedThreadId !== previous.selectedThreadId
        ) {
          useSidebarStore.getState().setNotesOpen(false)
        }
      }),
    []
  )

  return (
    <main className='flex min-w-0 flex-1 flex-col'>
      {noteId ? (
        <NoteEditor
          key={noteId}
          noteId={noteId}
          newProjectId={projectId}
          onMoved={(next) => {
            if (next !== null) list(next, noteId)
          }}
          onDeleted={close}
        />
      ) : (
        <>
          <NotesHeader>
            <h1 data-tauri-drag-region className='text-sm font-medium'>
              {m.notes()}
            </h1>
          </NotesHeader>
          <div className='text-muted-foreground flex flex-1 flex-col items-center justify-center gap-4 px-6 text-center text-sm'>
            <LogoMark className='text-muted-foreground/25 size-12' />
            <p className='max-w-sm text-pretty'>
              {m.notes_pick({ shortcut: QUICK_NOTE_SHORTCUT() })}
            </p>
            <Button
              variant='outline'
              size='sm'
              onClick={() => open(crypto.randomUUID())}
            >
              {m.notes_new()}
            </Button>
          </div>
        </>
      )}
    </main>
  )
}
