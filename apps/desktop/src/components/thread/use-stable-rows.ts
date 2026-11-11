import { useRef } from 'react'
import { stableRows } from '@/lib/stable-rows'
import type { TranscriptRow } from '@/lib/tool-calls'

/**
 * `rows` with the identity of the rows shown before wherever they didn't change, so their
 * memoized components skip rendering. It remembers what it showed, which React Compiler
 * can't follow, so it is left out of it.
 */
export function useStableRows(rows: TranscriptRow[]): TranscriptRow[] {
  'use no memo'
  const last = useRef<{
    rows: TranscriptRow[]
    shown: TranscriptRow[]
  } | null>(null)
  if (last.current?.rows !== rows) {
    last.current = { rows, shown: stableRows(last.current?.shown ?? [], rows) }
  }
  return last.current.shown
}
