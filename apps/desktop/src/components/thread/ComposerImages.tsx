import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import type { ComposerImage } from './use-composer-images'
import { Alert02Icon, Cancel01Icon } from '@hugeicons/core-free-icons'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'

type ComposerImagesProps = {
  images: ComposerImage[]
  onOpen: (key: string) => void
  onRemove: (key: string) => void
}

/** The draft's images, above the text; each opens large to check it. */
export function ComposerImages({
  images,
  onOpen,
  onRemove
}: ComposerImagesProps) {
  return (
    <ul className='flex flex-wrap gap-2 px-2 pt-1 pb-1.5'>
      {images.map((image) => (
        <li key={image.key} className='group relative'>
          <Tooltip>
            <TooltipTrigger
              render={
                <button
                  type='button'
                  aria-label={m.open_image({ name: image.name })}
                  onClick={() => onOpen(image.key)}
                  className={cn(
                    'border-border bg-muted focus-visible:ring-ring relative block size-14 cursor-zoom-in overflow-hidden rounded-lg border outline-none focus-visible:ring-2',
                    image.failed && 'border-destructive'
                  )}
                />
              }
            >
              <img
                src={image.previewUrl || undefined}
                alt={image.name}
                draggable={false}
                className={cn(
                  'size-full object-cover',
                  !image.saved && 'opacity-50'
                )}
              />
              {image.failed ? (
                <span className='bg-background/70 text-destructive absolute inset-0 grid place-items-center'>
                  <HugeiconsIcon icon={Alert02Icon} strokeWidth={2} />
                </span>
              ) : null}
            </TooltipTrigger>
            <TooltipContent>
              {image.failed
                ? m.composer_image_failed({ name: image.name })
                : image.name}
            </TooltipContent>
          </Tooltip>
          <button
            type='button'
            aria-label={m.composer_remove_image({ name: image.name })}
            onClick={() => onRemove(image.key)}
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
