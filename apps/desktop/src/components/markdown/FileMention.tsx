import { cn, formatFileSize } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { CHIP_CLASSES } from '@/lib/chips'
import type { ExtraProps } from 'react-markdown'
import { getLocale } from '@/paraglide/runtime'
import { openAttachment } from './open-local-file'
import { FileIcon } from '@/components/files/FileIcon'

/** A `[File #n]` chip in a sent message: its icon, name and size; opens the file. */
export function FileMention({ node }: ExtraProps) {
  const properties = node?.properties ?? {}
  const id = String(properties.dataId ?? '')
  const name = String(properties.dataName ?? '')
  const mimeType = String(properties.dataMimeType ?? '')
  const size = Number(properties.dataSize ?? 0)

  return (
    <button
      type='button'
      aria-label={m.open_attached_file({ name })}
      title={name}
      onClick={() => openAttachment({ type: 'file', id, name, mimeType, size })}
      className={cn(
        'cursor-pointer rounded-sm px-0.5',
        CHIP_CLASSES.attachedFile
      )}
    >
      <FileIcon
        path={name}
        className='me-1 inline-block size-3.5 align-[-2px]'
      />
      {name}
      {size > 0 ? (
        <span className='text-muted-foreground ms-1 text-[0.85em]'>
          {formatFileSize(size, getLocale())}
        </span>
      ) : null}
    </button>
  )
}
