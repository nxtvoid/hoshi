import type { TranscriptItem, TranscriptRow, TurnChanges } from './tool-calls'

export function rowKey(row: TranscriptRow) {
  return row.kind === 'item' ? row.item.id : row.id
}

/**
 * `next` with each row that did not change taken from `previous`, so unchanged rows keep
 * their identity and memoized rows skip rendering. `previous` itself when nothing changed.
 */
export function stableRows(
  previous: TranscriptRow[],
  next: TranscriptRow[]
): TranscriptRow[] {
  if (previous.length === 0) return next
  const byKey = new Map<string, TranscriptRow>()
  for (const row of previous) byKey.set(`${row.kind}:${rowKey(row)}`, row)
  let same = previous.length === next.length
  const rows = next.map((row, index) => {
    const kept = stableRow(byKey.get(`${row.kind}:${rowKey(row)}`), row)
    if (kept !== previous[index]) same = false
    return kept
  })
  return same ? previous : rows
}

function stableRow(
  old: TranscriptRow | undefined,
  row: TranscriptRow
): TranscriptRow {
  if (!old) return row
  if (old.kind === 'item' && row.kind === 'item') {
    return sameItem(old.item, row.item) && sameChanges(old.changes, row.changes)
      ? old
      : row
  }
  if (old.kind === 'work' && row.kind === 'work') {
    return old.entries.length === row.entries.length &&
      row.entries.every((entry, index) => sameItem(old.entries[index], entry))
      ? old
      : row
  }
  if (old.kind === 'fold' && row.kind === 'fold') {
    if (
      old.ending !== row.ending ||
      old.after !== row.after ||
      old.durationMs !== row.durationMs
    ) {
      return row
    }
    const rows = stableRows(old.rows, row.rows)
    return rows === old.rows ? old : { ...row, rows }
  }
  return row
}

/** Streamed items are rebuilt on every change, so they compare by what they hold. */
function sameItem(a: TranscriptItem | undefined, b: TranscriptItem) {
  if (a === b) return true
  if (!a?.streaming || !b.streaming || a.id !== b.id) return false
  const x = a.content
  const y = b.content
  return (
    (x.kind === 'assistantMessage' || x.kind === 'reasoning') &&
    (y.kind === 'assistantMessage' || y.kind === 'reasoning') &&
    x.kind === y.kind &&
    x.text === y.text
  )
}

function sameChanges(a: TurnChanges | undefined, b: TurnChanges | undefined) {
  return a === b || (a?.turnId === b?.turnId && a?.files === b?.files)
}
