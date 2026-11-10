import { useEffect, useState } from 'react'
import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@hoshi/ui/components/button'
import { showNote } from '@/state/notes-hub'
import { Markdown } from '@/components/markdown/Markdown'
import { useTranscriptStore } from '@/state/transcript-store'
import type { MessageAttachment } from '@/bindings/MessageAttachment'
import { hiddenCredentialsLabel } from '@/components/notes/note-labels'
import { readNote } from '@/state/notes-store'

type SentNote = Extract<MessageAttachment, { type: 'note' }>

/** The note as the message `itemId` carried it, while that message is loaded. */
export function useSentNote(itemId: string, noteId: string) {
  return useTranscriptStore((state) => {
    const item = state.items.find((candidate) => candidate.id === itemId)
    if (item?.content.kind !== 'userMessage') return undefined
    return item.content.attachments.find(
      (attachment): attachment is SentNote =>
        attachment.type === 'note' && attachment.id === noteId
    )
  })
}

/**
 * A note a message carried, beside the thread: what the agent read, without the
 * credentials it was sent without, shown as the markdown it is. A message too far back to
 * be loaded shows the note as it is now.
 */
export function NoteSurface({
  itemId,
  noteId
}: {
  itemId: string
  noteId: string
}) {
  const sent = useSentNote(itemId, noteId)
  const loading = useTranscriptStore((state) => state.loading)
  /** `null` once the note turns out to be gone. */
  const [current, setCurrent] = useState<SentNote | null | undefined>(undefined)
  const [gone, setGone] = useState(false)
  const missing = sent === undefined && !loading

  useEffect(() => {
    if (!missing) return
    let cancelled = false
    invoke<SentNote>('get_note_attachment', { noteId })
      .then((found) => {
        if (!cancelled) setCurrent(found)
      })
      .catch(() => {
        if (!cancelled) setCurrent(null)
      })
    return () => {
      cancelled = true
    }
  }, [missing, noteId])

  const note = sent ?? (loading ? undefined : current)
  if (note === undefined) return <div className='flex-1' />
  if (note === null) {
    return (
      <p className='text-muted-foreground flex-1 p-4 text-center text-xs'>
        {m.note_surface_gone()}
      </p>
    )
  }

  const edit = async () => {
    const now = await readNote(noteId)
    if (now) showNote(now)
    else setGone(true)
  }
  const caption = sent ? m.message_note_sent() : m.note_surface_current()

  return (
    <div className='flex min-h-0 flex-1 flex-col'>
      <div className='text-muted-foreground flex shrink-0 items-center gap-2 px-3.5 pt-2 pb-1 text-xs'>
        <span className='min-w-0 flex-1'>
          {note.hidden > 0
            ? `${caption} ${hiddenCredentialsLabel(note.hidden)}.`
            : caption}
        </span>
        {gone ? (
          <span className='shrink-0'>{m.note_surface_gone()}</span>
        ) : (
          <Button
            size='xs'
            variant='ghost'
            className='-me-1.5 shrink-0'
            onClick={() => void edit()}
          >
            {m.note_surface_edit()}
          </Button>
        )}
      </div>
      <div className='min-h-0 flex-1 overflow-auto px-3.5 pt-1 pb-4'>
        <Markdown text={note.text} source='forge' />
      </div>
    </div>
  )
}
