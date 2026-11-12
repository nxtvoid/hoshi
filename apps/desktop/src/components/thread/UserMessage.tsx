import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { cn, formatClockTime } from '@hoshi/utils'
import { getLocale } from '@/paraglide/runtime'
import { knownMentions } from '@/lib/prompt-segments'
import { useImageStore } from '@/state/image-store'
import { CopyButton } from '@hoshi/ui/components/copy-button'
import { Markdown } from '@/components/markdown/Markdown'
import type { MessageAttachment } from '@/bindings/MessageAttachment'
import { ImageDialog } from '@/components/markdown/ImageMention'
import { FileMention } from '@/components/markdown/FileMention'
import { ArrowTurnForwardIcon, Copy01Icon } from '@hugeicons/core-free-icons'
import {
  type ReactNode,
  useEffect,
  useLayoutEffect,
  useRef,
  useState
} from 'react'

type Image = Extract<MessageAttachment, { type: 'image' }>
type AttachedFile = Extract<MessageAttachment, { type: 'file' }>

/** The files whose `[File #n]` the text doesn't show, which show above it instead. */
function filesOutsideText(text: string, attachments: MessageAttachment[]) {
  const shown = new Set(
    Array.from(text.matchAll(/\[File #(\d+)\]/g), (match) => Number(match[1]))
  )
  return attachments
    .filter(
      (attachment): attachment is AttachedFile => attachment.type === 'file'
    )
    .filter((_, index) => !shown.has(index + 1))
}

type UserMessageProps = {
  text: string
  attachments: MessageAttachment[]
  /** Sent while the agent worked, to steer it. */
  steer?: boolean
  /** When it was sent, shown with its actions while it is hovered. */
  createdAt?: string
  /** Shown before the copy button while it is hovered, such as editing from here. */
  action?: ReactNode
}

export function UserMessage({
  text,
  attachments,
  steer,
  createdAt,
  action
}: UserMessageProps) {
  const images = attachments.filter(
    (attachment): attachment is Image => attachment.type === 'image'
  )
  const files = filesOutsideText(text, attachments)
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
      {files.length > 0 ? (
        <ul className='flex max-w-[85%] flex-wrap justify-end gap-1.5 text-sm'>
          {files.map((file) => (
            <li key={file.id}>
              <FileMention
                node={{
                  type: 'element',
                  tagName: 'span',
                  properties: {
                    dataId: file.id,
                    dataName: file.name,
                    dataMimeType: file.mimeType,
                    dataSize: file.size
                  },
                  children: []
                }}
              />
            </li>
          ))}
        </ul>
      ) : null}
      {text ? (
        <MessageBubble>
          <Markdown
            text={text}
            source='forge'
            mentions={knownMentions(attachments)}
          />
        </MessageBubble>
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

const COLLAPSED_HEIGHT = 320

function MessageBubble({ children }: { children: ReactNode }) {
  const content = useRef<HTMLDivElement>(null)
  const [overflows, setOverflows] = useState(false)
  const [expanded, setExpanded] = useState(false)

  useLayoutEffect(() => {
    const element = content.current
    if (!element) return
    const measure = () =>
      setOverflows(element.scrollHeight > COLLAPSED_HEIGHT + 48)
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  const collapsed = overflows && !expanded

  return (
    <div className='bg-muted flex max-w-[85%] flex-col rounded-xl px-3 py-2'>
      <div
        ref={content}
        className={cn(
          collapsed &&
            'max-h-80 overflow-hidden [mask-image:linear-gradient(to_bottom,black_calc(100%-4rem),transparent)]'
        )}
      >
        {children}
      </div>
      {overflows ? (
        <button
          type='button'
          aria-expanded={expanded}
          onClick={() => setExpanded(!expanded)}
          className='bg-background/70 text-foreground/80 hover:bg-background hover:text-foreground focus-visible:ring-ring mt-1 -mr-1 self-end rounded-md px-2 py-1 text-xs font-medium outline-none focus-visible:ring-2'
        >
          {expanded ? m.message_show_less() : m.message_show_full()}
        </button>
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
          <img src={url} alt={image.name} className='size-full object-cover' />
        ) : null}
      </button>
      <ImageDialog
        name={image.name}
        url={url}
        open={open}
        onOpenChange={setOpen}
      />
    </>
  )
}
