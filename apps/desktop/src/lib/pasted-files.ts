import { invoke } from '@tauri-apps/api/core'
import { attachmentMimeType, classifyAttachment } from './attachment-files'

/** What a paste or drop carries, as WebKit and Chromium hand it to the page. */
export type Transfer = {
  files: FileList | null
  items?: DataTransferItemList
  getData: (format: string) => string
}

/**
 * Files a paste or drop brings: files the page can read, paths of files (a Linux webview
 * gets a file manager's copied files as paths only), or what only the system clipboard
 * holds: an image as an email client copies it, or files a file manager copied.
 */
export type PastedFiles =
  | { kind: 'files'; files: File[] }
  | { kind: 'paths'; paths: string[]; text: string }
  | { kind: 'clipboard' }

const IMAGE_EXTENSIONS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp'])

const extensionOf = (path: string) =>
  /\.([a-z0-9]+)$/i.exec(path)?.[1]?.toLowerCase() ?? ''

function itemFiles(items: DataTransferItemList | undefined): File[] {
  return Array.from(items ?? []).flatMap((item) => {
    if (item.kind !== 'file') return []
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

function lines(text: string) {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line !== '' && !line.startsWith('#'))
}

/** The image files `text` names, one per line as paths or `file://` URIs; `null` if anything else is there. */
export function imagePaths(text: string): string[] | null {
  const found = lines(text)
  if (found.length === 0) return null
  const paths = found.map(asPath)
  const images = paths.every(
    (path) => path !== null && IMAGE_EXTENSIONS.has(extensionOf(path))
  )
  return images ? (paths as string[]) : null
}

/** The files a URI list names, all `file://` URIs; `null` if anything else is there. */
export function fileUris(uriList: string): string[] | null {
  const found = lines(uriList)
  if (
    found.length === 0 ||
    !found.every((line) => line.startsWith('file://'))
  ) {
    return null
  }
  const paths = found.map(asPath)
  return paths.every((path) => path !== null) ? (paths as string[]) : null
}

/** Whether HTML shows text, not only images. */
const showsText = (html: string) =>
  html
    .replace(/<[^>]*>/g, '')
    .replace(/&nbsp;/g, ' ')
    .trim() !== ''

/** A file the composer shows as an image: one the agent sees, or an image type it can't. */
const isImageLike = (file: File) =>
  classifyAttachment(file) !== 'file' || file.type.startsWith('image/')

/**
 * The files in a paste or drop, or `null` for one to leave to the editor. Images always go
 * as files; other files only when no text comes with them, since apps that copy text often
 * add a file of it. A paste with no text at all may still hold files only the system
 * clipboard gives, when `orClipboard`.
 */
export function pastedFiles(
  data: Transfer,
  { orClipboard }: { orClipboard: boolean }
): PastedFiles | null {
  const listed = Array.from(data.files ?? [])
  const files = listed.length > 0 ? listed : itemFiles(data.items)
  const text = data.getData('text/plain')
  if (files.some(isImageLike)) return { kind: 'files', files }
  if (files.length > 0 && text.trim() === '') return { kind: 'files', files }
  const paths = fileUris(data.getData('text/uri-list')) ?? imagePaths(text)
  if (paths) return { kind: 'paths', paths, text }
  const empty = text.trim() === '' && !showsText(data.getData('text/html'))
  return orClipboard && empty ? { kind: 'clipboard' } : null
}

function fileFrom(bytes: ArrayBuffer, path: string): File {
  const name = path.split(/[\\/]/).pop() || 'file'
  return new File([bytes], name, {
    type: attachmentMimeType({ name, type: '' })
  })
}

/** Files read from a paste, and the names of those that could not be read. */
export type ReadFiles = { files: File[]; unreadable: string[] }

const nameOf = (path: string) => path.split(/[\\/]/).pop() || path

/** Reads at most `limit` paths, one past the room left so the overflow is reported. */
async function readPaths(paths: string[], limit: number): Promise<ReadFiles> {
  const read = await Promise.all(
    paths.slice(0, Math.max(limit, 0) + 1).map((path) =>
      invoke<ArrayBuffer>('read_pasted_file', { path })
        .then((bytes) => fileFrom(bytes, path))
        .catch(() => nameOf(path))
    )
  )
  return {
    files: read.filter((file) => typeof file !== 'string'),
    unreadable: read.filter((file) => typeof file === 'string')
  }
}

/**
 * The files of `pasted`, read through hoshi where the page can't, for a draft with `limit`
 * slots left. `null` when no path is a file after all, so its text is pasted instead.
 */
export async function readPastedFiles(
  pasted: PastedFiles,
  limit: number
): Promise<ReadFiles | null> {
  switch (pasted.kind) {
    case 'files':
      return { files: pasted.files, unreadable: [] }
    case 'paths': {
      const read = await readPaths(pasted.paths, limit)
      return read.files.length > 0 ? read : null
    }
    case 'clipboard': {
      const copied = await invoke<string[]>('read_clipboard_files').catch(
        () => []
      )
      if (copied.length > 0) return readPaths(copied, limit)
      const bytes = await invoke<ArrayBuffer>('read_clipboard_image').catch(
        () => null
      )
      return {
        files: bytes ? [fileFrom(bytes, 'image.png')] : [],
        unreadable: []
      }
    }
  }
}
