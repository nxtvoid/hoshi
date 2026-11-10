import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import type { AttachedNote } from '@/state/notes-store'
import { Cancel01Icon, Note01Icon } from '@hugeicons/core-free-icons'
import { hiddenCredentialsLabel } from '@/components/notes/note-labels'

type ComposerNotesProps = {
  notes: AttachedNote[]
  onRemove: (noteId: string) => void
}

/** The notes the draft carries, above the text. The agent reads them with this message only. */
export function ComposerNotes({ notes, onRemove }: ComposerNotesProps) {
  return (
    <ul className='flex flex-wrap gap-1.5 px-2 pt-1 pb-1.5'>
      {notes.map((note) => {
        const title = note.title || m.notes_untitled()
        return (
          <li
            key={note.id}
            className='border-border bg-background flex h-7 max-w-full items-center gap-1.5 rounded-lg border ps-2 pe-1 text-xs'
          >
            <HugeiconsIcon
              icon={Note01Icon}
              strokeWidth={2}
              className='text-muted-foreground size-3.5 shrink-0'
            />
            <span className='min-w-0 truncate'>{title}</span>
            {note.credentials > 0 ? (
              <span className='text-status-waiting shrink-0'>
                {hiddenCredentialsLabel(note.credentials)}
              </span>
            ) : null}
            <button
              type='button'
              aria-label={m.composer_remove_note({ title })}
              onClick={() => onRemove(note.id)}
              className='text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:ring-ring grid size-5 shrink-0 place-items-center rounded-md outline-none focus-visible:ring-2'
            >
              <HugeiconsIcon
                icon={Cancel01Icon}
                strokeWidth={2}
                className='size-3'
              />
            </button>
          </li>
        )
      })}
    </ul>
  )
}
