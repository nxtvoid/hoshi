import { memo } from 'react'
import { cn } from '@hoshi/utils'
import { fileIcon } from '@/lib/file-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { getBuiltInSpriteSheet } from '@pierre/trees'
import { Folder01Icon } from '@hugeicons/core-free-icons'

const SPRITE_ID = 'hoshi-file-icon-sprite'

/** Every icon points into one sprite, put in the document once. */
function ensureSprite() {
  if (typeof document === 'undefined' || document.getElementById(SPRITE_ID)) {
    return
  }
  const container = document.createElement('div')
  container.id = SPRITE_ID
  container.setAttribute('aria-hidden', 'true')
  container.style.cssText =
    'position:absolute;width:0;height:0;overflow:hidden;pointer-events:none'
  container.innerHTML = getBuiltInSpriteSheet('complete')
  document.body.prepend(container)
}

ensureSprite()

/** A file's icon, colored by its kind; folders get a plain folder. */
export const FileIcon = memo(function FileIcon({
  path,
  directory = false,
  className
}: {
  path: string
  directory?: boolean
  className?: string
}) {
  if (directory) {
    return (
      <HugeiconsIcon
        icon={Folder01Icon}
        strokeWidth={2}
        className={cn('text-muted-foreground size-4 shrink-0', className)}
      />
    )
  }
  const { name, color } = fileIcon(path)
  return (
    <svg
      aria-hidden='true'
      viewBox='0 0 16 16'
      className={cn('size-4 shrink-0', color, className)}
    >
      <use href={`#${name}`} />
    </svg>
  )
})
