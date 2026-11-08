import type { TranscriptRow } from './tool-calls'

/** One mark per message the user sent, with what the agent last answered to it. */
export type MinimapItem = {
  id: string
  userText: string
  assistantText: string | null
}

/** Fewer messages than this need no map. */
export const MINIMAP_MIN_ITEMS = 2

/** Room between two marks, until the rail fills the height it may take. */
export const MINIMAP_SPACING_PX = 8

export function minimapItems(rows: TranscriptRow[]): MinimapItem[] {
  const items: MinimapItem[] = []
  for (const row of rows) {
    if (row.kind !== 'item') continue
    const { content } = row.item
    if (content.kind === 'userMessage') {
      items.push({
        id: row.item.id,
        userText: content.text,
        assistantText: null
      })
    } else if (content.kind === 'assistantMessage') {
      const last = items.at(-1)
      if (last) last.assistantText = content.text
    }
  }
  return items
}

/** Where mark `index` sits on the rail, from 0 at its top to 100 at its bottom. */
export function minimapTopPercent(index: number, count: number) {
  if (count <= 1) return 0
  return (Math.max(0, Math.min(index, count - 1)) / (count - 1)) * 100
}

/** The mark nearest the pointer on a rail that starts at `top` and is `height` tall. */
export function minimapIndexAt(
  pointerY: number,
  top: number,
  height: number,
  count: number
): number | null {
  if (count <= 0 || height <= 0) return null
  if (count === 1) return 0
  const progress = Math.max(0, Math.min(1, (pointerY - top) / height))
  return Math.round(progress * (count - 1))
}

/**
 * The message being read: the first one on screen, or else the last one above it, whose
 * answer fills the screen. `bounds` are each message's top and bottom, in view order.
 */
export function minimapCurrentIndex(
  bounds: { top: number; bottom: number }[],
  viewTop: number,
  viewBottom: number
): number | null {
  let above: number | null = null
  for (const [index, { top, bottom }] of bounds.entries()) {
    if (bottom > viewTop && top < viewBottom) return index
    if (top < viewTop) above = index
  }
  return above
}

/** Markdown as one line of plain text, for a preview. */
export function plainPreview(markdown: string, limit = 1500) {
  return markdown
    .slice(0, limit)
    .replace(/```[^\n]*\n?/g, ' ')
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/^\s{0,3}(#{1,6}|>|[-*+]|\d+\.)\s+/gm, '')
    .replace(/[*_~`]+/g, '')
    .replace(/\s+/g, ' ')
    .trim()
}
