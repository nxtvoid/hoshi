/** The most images and files one message carries. */
export const MAX_ATTACHMENTS = 100
/** The largest file one message carries. */
export const MAX_FILE_BYTES = 50 * 1024 * 1024
/** A paste this large, in UTF-8, is attached as a text file rather than typed in. */
export const PASTED_TEXT_ATTACHMENT_BYTES = 32 * 1024

/**
 * How a file joins the draft: as an image the agent sees, as a file it reads from disk, or
 * not at all, for an image type neither the agent nor the app can show.
 */
export type AttachmentKind = 'image' | 'file' | 'unsupportedImage'

/** Image types the app can draw and send as an image, redrawn as PNG when needed. */
const DRAWABLE_IMAGES = new Set([
  'image/png',
  'image/jpeg',
  'image/gif',
  'image/webp',
  'image/bmp',
  'image/avif',
  'image/x-icon',
  'image/vnd.microsoft.icon'
])

/** Image types worth sending as files: the agent reads them as text or by their bytes. */
const FILE_IMAGES = new Set([
  'image/svg+xml',
  'image/tiff',
  'image/dng',
  'image/x-dng',
  'image/x-adobe-dng'
])

const IMAGE_TYPE_BY_EXTENSION: Record<string, string> = {
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  gif: 'image/gif',
  webp: 'image/webp',
  bmp: 'image/bmp'
}

const FILE_TYPE_BY_EXTENSION: Record<string, string> = {
  svg: 'image/svg+xml',
  dng: 'image/x-adobe-dng',
  tif: 'image/tiff',
  tiff: 'image/tiff',
  pdf: 'application/pdf',
  txt: 'text/plain',
  md: 'text/markdown',
  csv: 'text/csv',
  json: 'application/json',
  docx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  xlsx: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  pptx: 'application/vnd.openxmlformats-officedocument.presentationml.presentation'
}

type NamedFile = Pick<File, 'name' | 'type'>

const extensionOf = (name: string) => {
  const dot = name.lastIndexOf('.')
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : ''
}

const essence = (type: string) => type.split(';')[0]?.trim().toLowerCase() ?? ''

/** Drags from other apps and files read by path can come with no type, or a generic one. */
const untyped = (file: NamedFile) => {
  const type = essence(file.type)
  return type === '' || type === 'application/octet-stream'
}

/** The type a file is sent as: its own, or the one its name says when it has none. */
export function attachmentMimeType(file: NamedFile): string {
  const extension = extensionOf(file.name)
  if (extension === 'svg') return 'image/svg+xml'
  if (extension === 'dng') return 'image/x-adobe-dng'
  if (!untyped(file)) return file.type
  return (
    IMAGE_TYPE_BY_EXTENSION[extension] ??
    FILE_TYPE_BY_EXTENSION[extension] ??
    'application/octet-stream'
  )
}

export function classifyAttachment(file: NamedFile): AttachmentKind {
  const type = essence(attachmentMimeType(file))
  if (FILE_IMAGES.has(type)) return 'file'
  if (!type.startsWith('image/')) return 'file'
  return DRAWABLE_IMAGES.has(type) ? 'image' : 'unsupportedImage'
}

/** `file` with the type it is sent as, so the image pipeline reads a typed file. */
export function withAttachmentType(file: File): File {
  const type = attachmentMimeType(file)
  if (type === file.type) return file
  return new File([file], file.name, { type, lastModified: file.lastModified })
}

/** Why `file` can't join a draft that already carries `count` attachments; `null` if it can. */
export type AttachmentProblem =
  | { kind: 'tooMany'; max: number }
  | { kind: 'unsupportedImage'; name: string }
  | { kind: 'empty'; name: string }
  | { kind: 'tooLarge'; name: string; max: number }

export function attachmentProblem(
  file: Pick<File, 'name' | 'type' | 'size'>,
  count: number
): AttachmentProblem | null {
  if (count >= MAX_ATTACHMENTS) return { kind: 'tooMany', max: MAX_ATTACHMENTS }
  const kind = classifyAttachment(file)
  if (kind === 'unsupportedImage') {
    return { kind: 'unsupportedImage', name: file.name }
  }
  if (kind === 'file') {
    if (file.size <= 0) return { kind: 'empty', name: file.name }
    if (file.size > MAX_FILE_BYTES) {
      return { kind: 'tooLarge', name: file.name, max: MAX_FILE_BYTES }
    }
  }
  return null
}

const KIB = 1024
const MIB = 1024 * 1024

/** A size limit the way it was set: `50 MB`, `512 KB`, or bytes. */
export function formatAttachmentLimit(bytes: number): string {
  if (bytes >= MIB && bytes % MIB === 0) return `${bytes / MIB} MB`
  if (bytes >= KIB && bytes % KIB === 0) return `${bytes / KIB} KB`
  return `${bytes} ${bytes === 1 ? 'byte' : 'bytes'}`
}

const encoder = new TextEncoder()

/** Whether pasted text is large enough to attach as a file rather than type in. */
export function attachesPastedText(text: string): boolean {
  return (
    text.length >= PASTED_TEXT_ATTACHMENT_BYTES ||
    encoder.encode(text).byteLength >= PASTED_TEXT_ATTACHMENT_BYTES
  )
}

/** `pasted-text.txt`, or the first `pasted-text-<n>.txt` none of `taken` has. */
export function pastedTextFileName(taken: readonly string[]): string {
  const names = new Set(taken.map((name) => name.toLowerCase()))
  if (!names.has('pasted-text.txt')) return 'pasted-text.txt'
  for (let sequence = 2; ; sequence++) {
    const name = `pasted-text-${sequence}.txt`
    if (!names.has(name)) return name
  }
}

/** Ctrl+Shift+V, or Cmd+Shift+V on a Mac: paste even a large text as text. */
export function isPasteAsTextShortcut(
  event: Pick<
    KeyboardEvent,
    'key' | 'code' | 'metaKey' | 'ctrlKey' | 'altKey' | 'shiftKey'
  >,
  mac: boolean
): boolean {
  const key = event.key.toLowerCase()
  const isV = /^[a-z]$/.test(key) ? key === 'v' : event.code === 'KeyV'
  return (
    isV &&
    event.shiftKey &&
    !event.altKey &&
    (mac ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey)
  )
}
