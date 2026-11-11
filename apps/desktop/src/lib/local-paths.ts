const WINDOWS_PATH = /^[a-z]:[\\/]/i
const POSIX_PATH = /^\/(?!\/)/
const LINE_SUFFIX = /:\d+(?::\d+)?$/
/** Files the system opens better than a code editor. */
const MEDIA =
  /\.(?:png|jpe?g|gif|webp|avif|svg|bmp|ico|pdf|html?|mp4|mov|webm|mp3|wav)$/i

function decoded(text: string) {
  try {
    return decodeURIComponent(text)
  } catch {
    return text
  }
}

/**
 * The file on this machine a link or image of an agent's answer names: an absolute path or
 * a `file:` URL, without the `:line` an agent may add. `null` for anything else.
 */
export function localPath(url: string | undefined): string | null {
  if (!url || /[\n\r]/.test(url)) return null
  let path: string
  if (/^file:\/\//i.test(url)) {
    path = decoded(url.slice('file://'.length))
    if (/^\/[a-z]:\//i.test(path)) path = path.slice(1)
  } else if (/^(?:[a-z]:|\/(?!\/))/i.test(url)) {
    path = decoded(url)
  } else {
    return null
  }
  if (!WINDOWS_PATH.test(path) && !POSIX_PATH.test(path)) return null
  return path.replace(LINE_SUFFIX, '')
}

/** A `file:` URL for `path`, which markdown keeps where it drops a bare `C:\` path. */
export function fileUrl(path: string) {
  const slashed = path.replaceAll('\\', '/')
  return `file://${slashed.startsWith('/') ? '' : '/'}${encodeURI(slashed)}`
}

/** Whether a code editor opens the file, rather than the system's app for it. */
export function opensInEditor(path: string) {
  return !MEDIA.test(path)
}
