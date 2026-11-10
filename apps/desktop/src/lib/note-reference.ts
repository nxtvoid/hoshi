const PREFIX = 'hoshi://note/'
const NOTE_ID = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i

/** What the user copies from a note and pastes into a message to attach it. */
export function noteReference(noteId: string) {
  return `${PREFIX}${noteId}`
}

/**
 * The notes a pasted text refers to, each once and in order; `null` when the text holds
 * anything other than references, so it is pasted as the text it is.
 */
export function noteReferences(text: string): string[] | null {
  const words = text.split(/\s+/).filter(Boolean)
  if (words.length === 0) return null
  const ids: string[] = []
  for (const word of words) {
    const id = word.startsWith(PREFIX) ? word.slice(PREFIX.length) : ''
    if (!NOTE_ID.test(id)) return null
    if (!ids.includes(id)) ids.push(id)
  }
  return ids
}
