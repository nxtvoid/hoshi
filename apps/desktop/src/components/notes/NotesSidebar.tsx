import { useEffect, useRef, useState } from 'react'
import { cn, formatRelativeTime } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { getLocale } from '@/paraglide/runtime'
import { type ListedNotes, withSavedNote } from '@/lib/note-list'
import { HugeiconsIcon } from '@hugeicons/react'
import { useAppStore } from '@/state/app-store'
import type { NoteSummary } from '@/bindings/NoteSummary'
import { QUICK_NOTE_SHORTCUT } from './quick-note-shortcut'
import { useNotesProjectId } from '@/state/notes-hub'
import { useSidebarStore } from '@/state/sidebar-store'
import { listNotes, useNotesStore } from '@/state/notes-store'
import {
  Alert02Icon,
  ArrowLeft01Icon,
  SquareLock02Icon
} from '@hugeicons/core-free-icons'

/** Quiet time after the last key before the search runs. */
const SEARCH_DELAY_MS = 120

/**
 * The notes in the sidebar, in place of the threads: the ones of the project the user
 * came from, or of the note they asked to see, then the general ones, the latest edited
 * first. The sidebar's search narrows them by their text.
 */
export function NotesSidebar() {
  const query = useNotesStore((state) => state.query)
  const revision = useNotesStore((state) => state.revision)
  const open = useNotesStore((state) => state.open)
  const projectId = useNotesProjectId()
  const openNoteId = useNotesStore((state) => state.openNoteId)
  const projectTitle = useAppStore(
    (state) => state.projects.find((project) => project.id === projectId)?.title
  )
  const lastSaved = useNotesStore((state) => state.lastSaved)
  const [found, setFound] = useState<ListedNotes | null>(null)
  /** Goes up to read the list again, as when a save may have changed what a search finds. */
  const [reads, setReads] = useState(0)
  const searching = useRef(false)
  searching.current = query.trim() !== ''
  const notes = found?.projectId === projectId ? found : null
  const groups = [
    { key: 'project', title: projectTitle, notes: notes?.own ?? [] },
    { key: 'general', title: m.notes_general(), notes: notes?.general ?? [] }
  ].filter((group) => group.notes.length > 0)

  useEffect(() => {
    let cancelled = false
    const timer = setTimeout(
      () => {
        Promise.all([
          projectId === null ? [] : listNotes(projectId, query),
          listNotes(null, query)
        ])
          .then(([own, general]) => {
            if (!cancelled) setFound({ projectId, own, general })
          })
          .catch((error) => console.error('hoshi: failed to list notes', error))
      },
      query ? SEARCH_DELAY_MS : 0
    )
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [projectId, query, revision, reads])

  useEffect(() => {
    if (!lastSaved) return
    if (searching.current) setReads((count) => count + 1)
    else setFound((listed) => listed && withSavedNote(listed, lastSaved))
  }, [lastSaved])

  return (
    <div className='flex min-w-0 flex-1 flex-col pt-1.5'>
      <BackHeader projectTitle={projectTitle} />
      {notes === null ? (
        <div className='flex-1' />
      ) : groups.length === 0 ? (
        <div className='flex flex-1 flex-col gap-1 p-4 text-center text-xs'>
          <p className='font-medium'>
            {query.trim() ? m.notes_no_matches() : m.notes_empty()}
          </p>
          {query.trim() ? null : (
            <p className='text-muted-foreground text-pretty'>
              {m.notes_empty_description({ shortcut: QUICK_NOTE_SHORTCUT() })}
            </p>
          )}
        </div>
      ) : (
        <div className='flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto px-1.5 pb-2'>
          {groups.map((group) => (
            <section key={group.key}>
              <h2 className='text-muted-foreground flex items-center gap-2 px-2 pt-2 pb-1 text-[0.6875rem] font-semibold tracking-wide uppercase'>
                <span className='min-w-0 truncate'>{group.title}</span>
                <span className='ml-auto font-medium tabular-nums'>
                  {group.notes.length}
                </span>
              </h2>
              <ul className='flex flex-col gap-px'>
                {group.notes.map((note) => (
                  <li key={note.id}>
                    <NoteRow
                      note={note}
                      selected={note.id === openNoteId}
                      onSelect={open}
                    />
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </div>
      )}
      <p className='border-sidebar-border text-muted-foreground flex shrink-0 items-start gap-1.5 border-t px-3 py-2 text-[0.6875rem] text-pretty'>
        <HugeiconsIcon
          icon={SquareLock02Icon}
          strokeWidth={2}
          className='mt-px size-3 shrink-0'
        />
        {m.notes_private()}
      </p>
    </div>
  )
}

/** "Notes", after the way back to the threads the user came from. */
function BackHeader({ projectTitle }: { projectTitle: string | undefined }) {
  const view = useSidebarStore((state) => state.view)
  const setNotesOpen = useSidebarStore((state) => state.setNotesOpen)
  const from =
    view === 'inbox'
      ? m.inbox()
      : view === 'chats'
        ? m.chats()
        : (projectTitle ?? m.projects())

  return (
    <div className='flex h-8 shrink-0 items-center gap-1.5 pr-1 pl-2 text-sm font-semibold'>
      <button
        type='button'
        onClick={() => setNotesOpen(false)}
        className='text-muted-foreground hover:text-foreground focus-visible:ring-sidebar-ring flex max-w-1/2 min-w-0 shrink items-center gap-0.5 rounded-sm font-normal outline-none focus-visible:ring-2'
      >
        <HugeiconsIcon
          icon={ArrowLeft01Icon}
          strokeWidth={2}
          className='size-3.5 shrink-0'
        />
        <span className='truncate'>{from}</span>
      </button>
      <span aria-hidden className='text-muted-foreground/60 font-normal'>
        /
      </span>
      <span className='truncate'>{m.notes()}</span>
    </div>
  )
}

function NoteRow({
  note,
  selected,
  onSelect
}: {
  note: NoteSummary
  selected: boolean
  onSelect: (noteId: string) => void
}) {
  return (
    <button
      type='button'
      aria-current={selected || undefined}
      onClick={() => onSelect(note.id)}
      className={cn(
        'hover:bg-sidebar-accent/60 focus-visible:ring-sidebar-ring flex w-full flex-col gap-0.5 rounded-md px-2 py-1.5 text-left outline-none focus-visible:ring-2',
        selected && 'bg-sidebar-accent text-sidebar-accent-foreground'
      )}
    >
      <span className='flex min-w-0 items-center gap-2'>
        <span
          className={cn(
            'min-w-0 flex-1 truncate text-sm',
            !note.title && 'text-muted-foreground'
          )}
        >
          {note.title || m.notes_untitled()}
        </span>
        {note.credentials > 0 ? (
          <HugeiconsIcon
            icon={Alert02Icon}
            strokeWidth={2}
            role='img'
            aria-label={m.notes_credential_badge()}
            className='text-status-waiting size-3 shrink-0'
          />
        ) : null}
        <span className='text-muted-foreground shrink-0 text-xs tabular-nums'>
          {formatRelativeTime(note.updatedAt, new Date(), getLocale())}
        </span>
      </span>
      {note.preview ? (
        <span className='text-muted-foreground truncate text-xs'>
          {note.preview}
        </span>
      ) : null}
    </button>
  )
}
