import { type ReactNode, useEffect, useState } from 'react'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { formatClockTime } from '@hoshi/utils'
import { getLocale } from '@/paraglide/runtime'
import { CopyButton } from '@hoshi/ui/components/copy-button'
import {
  ArrowTurnForwardIcon,
  Copy01Icon,
  Note01Icon
} from '@hugeicons/core-free-icons'
import { knownMentions } from '@/lib/prompt-segments'
import { useImageStore } from '@/state/image-store'
import type { MessageAttachment } from '@/bindings/MessageAttachment'
import { Dialog, DialogContent, DialogTitle } from '@hoshi/ui/components/dialog'
import { Markdown } from '@/components/markdown/Markdown'

type Image = Extract<MessageAttachment, { type: 'image' }>
type SentNote = Extract<MessageAttachment, { type: 'note' }>

type UserMessageProps = {
  text: string
  attachments: MessageAttachment[]
  /** Sent while the agent worked, to steer it. */
  steer?: boolean
  /** When it was sent, shown with its actions while it is hovered. */
  createdAt?: string
  /** Shown before the copy button while it is hovered, such as editing from here. */
  action?: ReactNode
  /** Shows a note the message carried, as the agent read it. */
  onOpenNote?: (noteId: string) => void
}

export function UserMessage({
  text,
  attachments,
  steer,
  createdAt,
  action,
  onOpenNote
}: UserMessageProps) {
  const images = attachments.filter(
    (attachment): attachment is Image => attachment.type === 'image'
  )
  const notes = attachments.filter(
    (attachment): attachment is SentNote => attachment.type === 'note'
  )
  const time = createdAt ? formatClockTime(createdAt, getLocale()) : ''

  return (
    <div className='group/message flex flex-col items-end gap-1.5'>
      {steer ? (
        <span className='text-muted-foreground flex items-center gap-1 text-xs'>
          <HugeiconsIcon
            icon={ArrowTurnForwardIcon}
            strokeWidth={2}
            className='size-3.5'
          />
          {m.message_steer()}
        </span>
      ) : null}
      {images.length > 0 ? (
        <ul className='flex max-w-[85%] flex-wrap justify-end gap-1.5'>
          {images.map((image) => (
            <li key={image.id}>
              <MessageImage image={image} />
            </li>
          ))}
        </ul>
      ) : null}
      {notes.length > 0 ? (
        <ul className='flex max-w-[85%] flex-wrap justify-end gap-1.5'>
          {notes.map((note) => (
            <li key={note.id} className='min-w-0'>
              <MessageNote
                note={note}
                onOpen={onOpenNote && (() => onOpenNote(note.id))}
              />
            </li>
          ))}
        </ul>
      ) : null}
      {text ? (
        <div className='bg-muted max-w-[85%] rounded-xl px-3 py-2'>
          <Markdown
            text={text}
            source='forge'
            mentions={knownMentions(attachments)}
          />
        </div>
      ) : null}
      {time || action || text ? (
        <div className='text-muted-foreground -mt-0.5 flex h-6 items-center gap-0.5 text-xs opacity-0 transition-opacity group-focus-within/message:opacity-100 group-hover/message:opacity-100'>
          {time ? <span className='px-1 tabular-nums'>{time}</span> : null}
          {action}
          {text ? (
            <CopyButton
              value={text}
              label={m.copy_message()}
              copiedLabel={m.copied()}
              icon={Copy01Icon}
              side='top'
              className='hover:text-foreground hover:bg-muted rounded-md p-1.5'
            />
          ) : null}
        </div>
      ) : null}
    </div>
  )
}

/** A note the message carried, which opens beside the thread as the agent read it. */
function MessageNote({
  note,
  onOpen
}: {
  note: SentNote
  onOpen: (() => void) | undefined
}) {
  const title = note.title || m.notes_untitled()

  return (
    <button
      type='button'
      aria-label={m.message_note_open({ title })}
      disabled={!onOpen}
      onClick={onOpen}
      className='border-border bg-background hover:bg-muted focus-visible:ring-ring flex h-7 max-w-full items-center gap-1.5 rounded-lg border px-2 text-xs outline-none focus-visible:ring-2'
    >
      <HugeiconsIcon
        icon={Note01Icon}
        strokeWidth={2}
        className='text-muted-foreground size-3.5 shrink-0'
      />
      <span className='min-w-0 truncate'>{title}</span>
    </button>
  )
}

function MessageImage({ image }: { image: Image }) {
  const url = useImageStore((state) => state.urls[image.id])
  const load = useImageStore((state) => state.load)
  const [open, setOpen] = useState(false)

  useEffect(() => {
    load(image.id, image.mimeType)
  }, [image.id, image.mimeType, load])

  return (
    <>
      <button
        type='button'
        aria-label={m.open_image({ name: image.name })}
        onClick={() => setOpen(true)}
        disabled={!url}
        className='border-border bg-muted block size-24 overflow-hidden rounded-lg border'
      >
        {url ? (
          <img
            src={url}
            alt={image.name}
            loading='lazy'
            className='size-full object-cover'
          />
        ) : null}
      </button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className='w-auto max-w-[90vw] p-2 sm:max-w-[90vw]'>
          <DialogTitle className='sr-only'>{image.name}</DialogTitle>
          {url ? (
            <img
              src={url}
              alt={image.name}
              className='max-h-[85vh] max-w-full rounded-lg object-contain'
            />
          ) : null}
        </DialogContent>
      </Dialog>
    </>
  )
}
