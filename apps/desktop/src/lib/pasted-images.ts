import { invoke } from '@tauri-apps/api/core'
import { imageFiles } from './images'

/** What a paste or drop carries, as WebKit and Chromium hand it to the page. */
export type Transfer = {
  files: FileList | null
  items?: DataTransferItemList
  getData: (format: string) => string
}

/**
 * Images a paste or drop brings: files the page can read, paths of image files (a Linux
 * webview gets a file manager's copied files as paths only), or an image only the system
 * clipboard holds, as an email client copies it.
 */
export type PastedImages =
  | { kind: 'files'; files: File[] }
  | { kind: 'paths'; paths: string[]; text: string }
  | { kind: 'clipboard' }

const IMAGE_TYPES: Record<string, string> = {
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  gif: 'image/gif',
  webp: 'image/webp',
  bmp: 'image/bmp'
}

const extensionOf = (path: string) =>
  /\.([a-z0-9]+)$/i.exec(path)?.[1]?.toLowerCase() ?? ''

function itemFiles(items: DataTransferItemList | undefined): File[] {
  return Array.from(items ?? []).flatMap((item) => {
    if (item.kind !== 'file' || !item.type.startsWith('image/')) return []
    const file = item.getAsFile()
    return file ? [file] : []
  })
}

function asPath(line: string): string | null {
  if (line.startsWith('file://')) {
    try {
      const path = decodeURIComponent(new URL(line).pathname)
      return /^\/[a-z]:\//i.test(path) ? path.slice(1) : path
    } catch {
      return null
    }
  }
  return line.startsWith('/') || /^[a-z]:[\\/]/i.test(line) ? line : null
}

/** The image files `text` names, one per line as paths or `file://` URIs; `null` if anything else is there. */
export function imagePaths(text: string): string[] | null {
  const lines = text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line !== '' && !line.startsWith('#'))
  if (lines.length === 0) return null
  const paths = lines.map(asPath)
  const images = paths.every(
    (path) => path !== null && extensionOf(path) in IMAGE_TYPES
  )
  return images ? (paths as string[]) : null
}

/** Whether HTML shows text, not only images. */
const showsText = (html: string) =>
  html
    .replace(/<[^>]*>/g, '')
    .replace(/&nbsp;/g, ' ')
    .trim() !== ''

/**
 * The images in a paste or drop, or `null` for one to leave to the editor. A paste with no
 * text at all may still hold an image only the system clipboard gives, when `orClipboard`.
 */
export function pastedImages(
  data: Transfer,
  { orClipboard }: { orClipboard: boolean }
): PastedImages | null {
  const files = imageFiles(data)
  const found = files.length > 0 ? files : itemFiles(data.items)
  if (found.length > 0) return { kind: 'files', files: found }
  const text = data.getData('text/plain')
  const paths = imagePaths(data.getData('text/uri-list')) ?? imagePaths(text)
  if (paths) return { kind: 'paths', paths, text }
  const empty = text.trim() === '' && !showsText(data.getData('text/html'))
  return orClipboard && empty ? { kind: 'clipboard' } : null
}

function fileFrom(bytes: ArrayBuffer, path: string): File {
  const name = path.split(/[\\/]/).pop() || 'image.png'
  return new File([bytes], name, {
    type: IMAGE_TYPES[extensionOf(name)] ?? 'image/png'
  })
}

/**
 * The files of `pasted`, read through hoshi where the page can't. `null` when a path is no
 * image file after all, so its text is pasted instead.
 */
export async function readPastedImages(
  pasted: PastedImages
): Promise<File[] | null> {
  switch (pasted.kind) {
    case 'files':
      return pasted.files
    case 'paths': {
      const read = await Promise.all(
        pasted.paths.map((path) =>
          invoke<ArrayBuffer>('read_image_file', { path })
            .then((bytes) => fileFrom(bytes, path))
            .catch(() => null)
        )
      )
      return read.every((file) => file !== null) ? read : null
    }
    case 'clipboard': {
      const bytes = await invoke<ArrayBuffer>('read_clipboard_image').catch(
        () => null
      )
      return bytes ? [fileFrom(bytes, 'image.png')] : []
    }
  }
}
