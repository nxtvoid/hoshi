import { m } from '@/paraglide/messages'
import { cn, formatFileSize } from '@hoshi/utils'
import { HugeiconsIcon } from '@hugeicons/react'
import { getLocale } from '@/paraglide/runtime'
import { FileIcon } from '@/components/files/FileIcon'
import type { ComposerAttachment } from './use-composer-attachments'
import { Alert02Icon, Cancel01Icon } from '@hugeicons/core-free-icons'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'

type ComposerAttachmentsProps = {
  attachments: ComposerAttachment[]
  onOpen: (attachment: ComposerAttachment) => void
  onRemove: (key: string) => void
}

/** The draft's images, and its files without a chip in the text, above the text. */
export function ComposerAttachments({
  attachments,
  onOpen,
  onRemove
}: ComposerAttachmentsProps) {
  return (
    <ul className='flex flex-wrap gap-2 px-2 pt-1 pb-1.5'>
      {attachments.map((attachment) => (
        <li key={attachment.key} className='group relative'>
          <Tooltip>
            <TooltipTrigger
              render={
                <button
                  type='button'
                  aria-label={
                    attachment.kind === 'image'
                      ? m.open_image({ name: attachment.name })
                      : m.open_attached_file({ name: attachment.name })
                  }
                  onClick={() => onOpen(attachment)}
                  className={cn(
                    'border-border bg-muted focus-visible:ring-ring relative block h-14 overflow-hidden rounded-lg border outline-none focus-visible:ring-2',
                    attachment.kind === 'image'
                      ? 'w-14 cursor-zoom-in'
                      : 'cursor-pointer',
                    attachment.failed && 'border-destructive'
                  )}
                />
              }
            >
              {attachment.kind === 'image' ? (
                <img
                  src={attachment.previewUrl || undefined}
                  alt={attachment.name}
                  draggable={false}
                  className={cn(
                    'size-full object-cover',
                    !attachment.saved && 'opacity-50'
                  )}
                />
              ) : (
                <FileTile attachment={attachment} />
              )}
              {attachment.failed ? (
                <span className='bg-background/70 text-destructive absolute inset-0 grid place-items-center'>
                  <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
                </span>
              ) : null}
            </TooltipTrigger>
            <TooltipContent>
              {attachment.failed
                ? m.composer_attachment_failed({ name: attachment.name })
                : attachment.name}
            </TooltipContent>
          </Tooltip>
          <button
            type='button'
            aria-label={m.composer_remove_attachment({
              name: attachment.name
            })}
            onClick={() => onRemove(attachment.key)}
            className='bg-foreground text-background absolute -top-1.5 -right-1.5 grid size-5 place-items-center rounded-full opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100'
          >
            <HugeiconsIcon
              icon={Cancel01Icon}
              strokeWidth={2.5}
              className='size-3'
            />
          </button>
        </li>
      ))}
    </ul>
  )
}

function FileTile({ attachment }: { attachment: ComposerAttachment }) {
  return (
    <span
      className={cn(
        'flex h-full max-w-56 items-center gap-2 px-3 text-start',
        !attachment.saved && !attachment.failed && 'opacity-50'
      )}
    >
      <FileIcon path={attachment.name} className='size-5' />
      <span className='flex min-w-0 flex-col'>
        <span className='truncate text-xs font-medium'>{attachment.name}</span>
        <span className='text-muted-foreground text-[0.7rem]'>
          {formatFileSize(attachment.size, getLocale())}
        </span>
      </span>
    </span>
  )
}
