import { useEffect, useState } from 'react'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { invoke } from '@tauri-apps/api/core'
import { openUrl } from '@tauri-apps/plugin-opener'
import { localPath } from '@/lib/local-paths'
import { openLocalFile } from './open-local-file'
import { Image01Icon } from '@hugeicons/core-free-icons'
import type { ComponentProps } from 'react'

/** Local images read so far, so a row that renders again doesn't read its image again. */
const localImages = new Map<string, string | null>()
const KEPT_IMAGES = 40

/** An image the app can't load, such as one in a private repository, becomes a link to it. */
export function MarkdownImage({ src, ...props }: ComponentProps<'img'>) {
  const local = typeof src === 'string' ? localPath(src) : null
  return local ? (
    <LocalImage path={local} {...props} />
  ) : (
    <RemoteImage src={src} {...props} />
  )
}

/** An image on this machine, which the engine reads: the page may not load files itself. */
function LocalImage({
  path,
  alt,
  ...props
}: ComponentProps<'img'> & { path: string }) {
  const [data, setData] = useState(() => localImages.get(path))

  useEffect(() => {
    if (localImages.has(path)) return
    let current = true
    invoke<string | null>('get_local_image', { path })
      .catch(() => null)
      .then((loaded) => {
        if (localImages.size >= KEPT_IMAGES) {
          const [oldest] = localImages.keys()
          if (oldest !== undefined) localImages.delete(oldest)
        }
        localImages.set(path, loaded)
        if (current) setData(loaded)
      })
    return () => {
      current = false
    }
  }, [path])

  if (data === undefined) return null
  if (data === null) {
    return (
      <ImageLink name={alt} title={path} open={() => openLocalFile(path)} />
    )
  }
  return (
    <button
      type='button'
      title={path}
      onClick={() => openLocalFile(path)}
      className='inline max-w-full cursor-zoom-in align-middle'
    >
      <img alt={alt} src={data} className='inline max-w-full' {...props} />
    </button>
  )
}

function RemoteImage({ alt, src, ...props }: ComponentProps<'img'>) {
  const [failed, setFailed] = useState(false)
  if (failed && typeof src === 'string') {
    return <ImageLink name={alt} title={src} open={() => void openUrl(src)} />
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

function ImageLink({
  name,
  title,
  open
}: {
  name: string | undefined
  title: string
  open: () => void
}) {
  return (
    <button
      type='button'
      title={title}
      onClick={open}
      className='border-border bg-muted/40 text-muted-foreground hover:text-foreground focus-visible:ring-ring inline-flex max-w-full items-center gap-1.5 rounded-md border px-2 py-1 align-middle text-xs outline-none focus-visible:ring-2'
    >
      <HugeiconsIcon
        icon={Image01Icon}
        strokeWidth={2}
        className='size-3.5 shrink-0'
      />
      <span className='truncate'>{name || m.unloaded_image()}</span>
    </button>
  )
}
