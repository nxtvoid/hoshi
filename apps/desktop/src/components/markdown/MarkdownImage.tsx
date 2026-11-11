import { useState } from 'react'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { openUrl } from '@tauri-apps/plugin-opener'
import { Image01Icon } from '@hugeicons/core-free-icons'
import type { ComponentProps } from 'react'

/** An image the app can't load, such as one in a private repository, becomes a link to it. */
export function MarkdownImage({ alt, src, ...props }: ComponentProps<'img'>) {
  const [failed, setFailed] = useState(false)
  if (failed && typeof src === 'string') {
    return (
      <button
        type='button'
        title={src}
        onClick={() => void openUrl(src)}
        className='border-border bg-muted/40 text-muted-foreground hover:text-foreground focus-visible:ring-ring inline-flex max-w-full items-center gap-1.5 rounded-md border px-2 py-1 align-middle text-xs outline-none focus-visible:ring-2'
      >
        <HugeiconsIcon
          icon={Image01Icon}
          strokeWidth={2}
          className='size-3.5 shrink-0'
        />
        <span className='truncate'>{alt || m.unloaded_image()}</span>
      </button>
    )
  }
  return (
    <img
      alt={alt}
      src={src}
      onError={() => setFailed(true)}
      className='inline max-w-full align-middle'
      {...props}
    />
  )
}
