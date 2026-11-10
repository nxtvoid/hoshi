import type { WorktreeSetup } from '@/bindings/WorktreeSetup'

export type SetupFiles = {
  /** Relative, `/`-separated, without blanks or repeats. */
  files: string[]
  /** Lines refused as written, trimmed. */
  invalid: string[]
}

/**
 * A file to copy made relative with `/` separators, as the engine does: `null` when blank,
 * `undefined` when absolute or leading out of the project or into `.git`.
 */
export function setupFilePath(raw: string): string | null | undefined {
  const unified = raw.trim().replaceAll('\\', '/')
  if (!unified) return null
  if (unified.startsWith('/')) return undefined
  const segments = unified
    .split('/')
    .filter((segment) => segment !== '' && segment !== '.')
  const first = segments[0]
  if (first === undefined) return undefined
  if (/^[a-z]:/i.test(first) || first.toLowerCase() === '.git') return undefined
  if (segments.includes('..')) return undefined
  return segments.join('/')
}

/** One file per line, as the setup dialog takes them. */
export function parseSetupFiles(lines: string | string[]): SetupFiles {
  const files: string[] = []
  const invalid: string[] = []
  const all = typeof lines === 'string' ? lines.split(/\r?\n/) : lines
  for (const line of all) {
    const path = setupFilePath(line)
    if (path === undefined) invalid.push(line.trim())
    else if (path !== null && !files.includes(path)) files.push(path)
  }
  return { files, invalid }
}

const SHARE_VERSION = 1

/** The setup as text to paste into another hoshi. */
export function shareSetup(setup: WorktreeSetup): string {
  return JSON.stringify({
    hoshiWorktreeSetup: SHARE_VERSION,
    files: setup.files,
    command: setup.command,
    wait: setup.wait
  })
}

/** A setup shared with `shareSetup`, or `null` when the text isn't one. */
export function parseSharedSetup(text: string): WorktreeSetup | null {
  let value: unknown
  try {
    value = JSON.parse(text.trim())
  } catch {
    return null
  }
  if (typeof value !== 'object' || value === null) return null
  const shared = value as Record<string, unknown>
  if (shared.hoshiWorktreeSetup !== SHARE_VERSION) return null
  const files = shared.files ?? []
  const command = shared.command ?? ''
  const wait = shared.wait ?? false
  if (
    !Array.isArray(files) ||
    !files.every((file) => typeof file === 'string') ||
    typeof command !== 'string' ||
    typeof wait !== 'boolean'
  ) {
    return null
  }
  return { files, command, wait }
}
