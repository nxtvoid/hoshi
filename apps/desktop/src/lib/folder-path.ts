/** Both separators count on Windows, where `sep` is a backslash. */
function isSeparator(char: string, sep: string) {
  return char === sep || (sep === '\\' && char === '/')
}

export function isAbsolutePath(path: string, sep: string) {
  if (sep === '\\')
    return /^[A-Za-z]:[\\/]/.test(path) || path.startsWith('\\\\')
  return path.startsWith('/')
}

export function endsWithSeparator(path: string, sep: string) {
  return path.length > 0 && isSeparator(path[path.length - 1] ?? '', sep)
}

/** The folder being looked into, up to its last separator, and the name typed after it. */
export function splitFolderQuery(query: string, sep: string) {
  for (let index = query.length - 1; index >= 0; index--) {
    if (isSeparator(query[index] ?? '', sep)) {
      return {
        directory: query.slice(0, index + 1),
        name: query.slice(index + 1)
      }
    }
  }
  return { directory: '', name: query }
}

export function joinPath(directory: string, name: string, sep: string) {
  if (!directory) return name
  return endsWithSeparator(directory, sep)
    ? `${directory}${name}`
    : `${directory}${sep}${name}`
}

export function withTrailingSeparator(path: string, sep: string) {
  return endsWithSeparator(path, sep) ? path : `${path}${sep}`
}

/** The path a typed query names: its folder when it ends in a separator. */
export function resolveFolderQuery(query: string, sep: string) {
  const trimmed = query.trim()
  if (!endsWithSeparator(trimmed, sep)) return trimmed
  const stripped = trimmed.slice(0, -1)
  return /^[A-Za-z]:$/.test(stripped) || stripped === '' ? trimmed : stripped
}

/** `~` at the start stands for the home folder. */
export function expandHome(query: string, home: string | null, sep: string) {
  if (!home || !query.startsWith('~')) return query
  if (query === '~') return withTrailingSeparator(home, sep)
  if (!isSeparator(query[1] ?? '', sep)) return query
  return joinPath(home, query.slice(2), sep)
}

/**
 * Folders whose name starts with `name` first, then those that contain it. Hidden ones
 * (a leading dot) only once the name starts with a dot.
 */
export function filterFolders(folders: string[], name: string) {
  const needle = name.toLowerCase()
  const visible = needle.startsWith('.')
    ? folders
    : folders.filter((folder) => !folder.startsWith('.'))
  if (!needle) return visible
  const starts = visible.filter((folder) =>
    folder.toLowerCase().startsWith(needle)
  )
  const contains = visible.filter(
    (folder) =>
      !folder.toLowerCase().startsWith(needle) &&
      folder.toLowerCase().includes(needle)
  )
  return [...starts, ...contains]
}

/** What every name starting with `name` has in common, for Tab to complete. */
export function completeFolderName(folders: string[], name: string) {
  const needle = name.toLowerCase()
  const matches = folders.filter((folder) =>
    folder.toLowerCase().startsWith(needle)
  )
  const first = matches[0]
  if (!first) return null
  let length = first.length
  for (const other of matches) {
    let shared = 0
    while (
      shared < length &&
      first[shared]?.toLowerCase() === other[shared]?.toLowerCase()
    ) {
      shared++
    }
    length = shared
  }
  return { name: first.slice(0, length), complete: matches.length === 1 }
}

/** The folder a clone of `url` gets by default, as git names it. Empty when there is none. */
export function repositoryFolderName(url: string) {
  const path = url
    .trim()
    .replace(/[?#].*$/, '')
    .replace(/[\\/]+$/, '')
  const last = path.split(/[\\/:]/).pop() ?? ''
  return last.replace(/\.git$/i, '')
}

const FORBIDDEN_NAME = /[\\/:*?"<>|]/

export function isValidFolderName(name: string) {
  const trimmed = name.trim()
  return (
    trimmed.length > 0 &&
    trimmed !== '.' &&
    trimmed !== '..' &&
    !FORBIDDEN_NAME.test(trimmed)
  )
}
