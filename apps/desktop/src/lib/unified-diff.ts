export type DiffLineKind = 'hunk' | 'added' | 'removed' | 'context' | 'note'

export type DiffLine = {
  kind: DiffLineKind
  text: string
  /** Line number in the old file; null for added lines and hunk headers. */
  oldLine: number | null
  /** Line number in the new file; null for removed lines and hunk headers. */
  newLine: number | null
}

const HUNK_HEADER = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)$/

/** Turns one file's `git diff` output into numbered lines, dropping the file headers. */
export function parseUnifiedDiff(text: string): DiffLine[] {
  const lines: DiffLine[] = []
  let oldLine = 0
  let newLine = 0
  let inHunk = false
  for (const raw of text.split('\n')) {
    const hunk = HUNK_HEADER.exec(raw)
    if (hunk) {
      inHunk = true
      oldLine = Number(hunk[1])
      newLine = Number(hunk[2])
      lines.push({ kind: 'hunk', text: raw, oldLine: null, newLine: null })
      continue
    }
    if (!inHunk) continue
    if (raw.startsWith('+')) {
      lines.push({ kind: 'added', text: raw.slice(1), oldLine: null, newLine })
      newLine += 1
    } else if (raw.startsWith('-')) {
      lines.push({
        kind: 'removed',
        text: raw.slice(1),
        oldLine,
        newLine: null
      })
      oldLine += 1
    } else if (raw.startsWith(' ')) {
      lines.push({ kind: 'context', text: raw.slice(1), oldLine, newLine })
      oldLine += 1
      newLine += 1
    } else if (raw.startsWith('\\')) {
      lines.push({ kind: 'note', text: raw, oldLine: null, newLine: null })
    } else if (raw.startsWith('diff --git')) {
      inHunk = false
    }
  }
  return lines
}
