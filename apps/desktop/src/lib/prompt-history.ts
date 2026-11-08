/** Which past prompt the composer shows, as an index into the history, oldest first. */
export type HistoryPosition = number | null

/**
 * Moves through past prompts: back from the newest, forward to an empty composer.
 * `null` when there is nowhere to go.
 */
export function stepHistory(
  entries: readonly string[],
  position: HistoryPosition,
  direction: 'back' | 'forward'
): { position: HistoryPosition; text: string } | null {
  if (direction === 'back') {
    const next = position === null ? entries.length - 1 : position - 1
    const text = entries[next]
    return text === undefined ? null : { position: next, text }
  }
  if (position === null) return null
  const text = entries[position + 1]
  return text === undefined
    ? { position: null, text: '' }
    : { position: position + 1, text }
}

/** Adds a sent prompt, dropping a repeat of the last one and the oldest past `limit`. */
export function remember(
  entries: readonly string[],
  prompt: string,
  limit: number
): string[] {
  const text = prompt.trim()
  if (text === '' || entries.at(-1) === text) return [...entries]
  return [...entries, text].slice(-limit)
}
