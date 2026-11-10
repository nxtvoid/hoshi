import { useEffect, useState } from 'react'
import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@hoshi/ui/components/button'
import { showNote } from '@/state/notes-hub'
import type { ThreadItem } from '@/bindings/ThreadItem'
import { Markdown } from '@/components/markdown/Markdown'
import { useTranscriptStore } from '@/state/transcript-store'
import type { MessageAttachment } from '@/bindings/MessageAttachment'
import { hiddenCredentialsLabel } from '@/components/notes/note-labels'
import { readNote } from '@/state/notes-store'

type SentNote = Extract<MessageAttachment, { type: 'note' }>

function sentNote(item: ThreadItem | null | undefined, noteId: string) {
  if (item?.content.kind !== 'userMessage') return undefined
  return item.content.attachments.find(
    (attachment): attachment is SentNote =>
      attachment.type === 'note' && attachment.id === noteId
  )
}

/** The note as the message `itemId` carried it, while that message is loaded. */
export function useSentNote(itemId: string, noteId: string) {
  return useTranscriptStore((state) =>
    sentNote(
      state.items.find((candidate) => candidate.id === itemId),
      noteId
    )
  )
}

/**
 * A note a message carried, beside the thread: what the agent read, without the
 * credentials it was sent without, shown as the markdown it is. The transcript leaves the
 * note's text out, so the message is read for it. A message too far back to be loaded
 * shows the note as it is now.
 */
export function NoteSurface({
  threadId,
  itemId,
  noteId
}: {
  threadId: string
  itemId: string
  noteId: string
}) {
  const sent = useSentNote(itemId, noteId)
  const loading = useTranscriptStore((state) => state.loading)
  /** The text the message carried, read from it; `null` when it can't be. */
  const [read, setRead] = useState<string | null | undefined>(undefined)
  /** `null` once the note turns out to be gone. */
  const [current, setCurrent] = useState<SentNote | null | undefined>(undefined)
  const [gone, setGone] = useState(false)
  const unread = sent !== undefined && sent.text === null
  const missing = (sent === undefined && !loading) || (unread && read === null)

  useEffect(() => {
    if (!unread) return
    let cancelled = false
    invoke<ThreadItem | null>('get_thread_item', { threadId, itemId })
      .then((item) => {
        if (!cancelled) setRead(sentNote(item, noteId)?.text ?? null)
      })
      .catch(() => {
        if (!cancelled) setRead(null)
      })
    return () => {
      cancelled = true
    }
  }, [unread, threadId, itemId, noteId])

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

  const carried = sent && (sent.text ?? read)
  const note = missing
    ? current
    : sent && typeof carried === 'string'
      ? { ...sent, text: carried }
      : undefined
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
  const caption = missing ? m.note_surface_current() : m.message_note_sent()

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
        <Markdown text={note.text ?? ''} source='forge' />
      </div>
    </div>
  )
}
