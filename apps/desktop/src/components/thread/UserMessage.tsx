import { type ReactNode, useEffect, useState } from 'react'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowTurnForwardIcon } from '@hugeicons/core-free-icons'
import { knownMentions } from '@/lib/prompt-segments'
import { useImageStore } from '@/state/image-store'
import type { MessageAttachment } from '@/bindings/MessageAttachment'
import { Dialog, DialogContent, DialogTitle } from '@hoshi/ui/components/dialog'
import { Markdown } from '@/components/markdown/Markdown'

type Image = Extract<MessageAttachment, { type: 'image' }>

type UserMessageProps = {
  text: string
  attachments: MessageAttachment[]
  /** Sent while the agent worked, to steer it. */
  steer?: boolean
  /** Shown beside the message while it is hovered. */
  action?: ReactNode
}

export function UserMessage({
  text,
  attachments,
  steer,
  action
}: UserMessageProps) {
  const images = attachments.filter(
    (attachment): attachment is Image => attachment.type === 'image'
  )

  return (
    <div className='flex flex-col items-end gap-1.5'>
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
      {text ? (
        <div className='group/message flex max-w-[85%] items-start justify-end gap-1'>
          {action ? (
            <span className='opacity-0 transition-opacity group-focus-within/message:opacity-100 group-hover/message:opacity-100'>
              {action}
            </span>
          ) : null}
          <div className='bg-muted min-w-0 rounded-xl px-3 py-2'>
            <Markdown
              text={text}
              source='forge'
              mentions={knownMentions(attachments)}
            />
          </div>
        </div>
      ) : null}
    </div>
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
