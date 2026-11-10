import { useEffect, useState } from 'react'
import { cn, formatFileSize } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { CHIP_CLASSES } from '@/lib/chips'
import type { ExtraProps } from 'react-markdown'
import { getLocale } from '@/paraglide/runtime'
import { useImageStore } from '@/state/image-store'
import { Dialog, DialogContent, DialogTitle } from '@hoshi/ui/components/dialog'

/** An `[Image #n]` chip in a sent message: its thumbnail, name and size; opens the image. */
export function ImageMention({ node }: ExtraProps) {
  const properties = node?.properties ?? {}
  const id = String(properties.dataId ?? '')
  const name = String(properties.dataName ?? '')
  const mimeType = String(properties.dataMimeType ?? '')
  const size = Number(properties.dataSize ?? 0)
  const url = useImageStore((state) => state.urls[id])
  const load = useImageStore((state) => state.load)
  const [open, setOpen] = useState(false)

  useEffect(() => {
    if (id) load(id, mimeType)
  }, [id, mimeType, load])

  return (
    <>
      <button
        type='button'
        aria-label={m.open_image({ name })}
        onClick={() => setOpen(true)}
        disabled={!url}
        className={cn(
          'cursor-pointer rounded-sm px-0.5 disabled:cursor-default',
          CHIP_CLASSES.image
        )}
      >
        {url ? (
          <img
            src={url}
            alt=''
            className='me-1 inline-block size-3.5 rounded-[3px] object-cover align-[-2px]'
          />
        ) : null}
        {name}
        {size > 0 ? (
          <span className='text-muted-foreground ms-1 text-[0.85em]'>
            {formatFileSize(size, getLocale())}
          </span>
        ) : null}
      </button>
      <ImageDialog name={name} url={url} open={open} onOpenChange={setOpen} />
    </>
  )
}

/** A message's image at full size. */
export function ImageDialog({
  name,
  url,
  open,
  onOpenChange
}: {
  name: string
  url: string | null | undefined
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className='w-auto max-w-[90vw] p-2 sm:max-w-[90vw]'>
        <DialogTitle className='sr-only'>{name}</DialogTitle>
        {url ? (
          <img
            src={url}
            alt={name}
            className='max-h-[85vh] max-w-full rounded-lg object-contain'
          />
        ) : null}
      </DialogContent>
    </Dialog>
  )
}
