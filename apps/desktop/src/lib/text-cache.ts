/**
 * Values by the text they were made from. Once the texts add up to more than `limit`
 * characters, the least recently used go; a text longer than `limit` is never kept.
 */
export function textCache<T>(limit: number) {
  const entries = new Map<string, T>()
  let size = 0
  return {
    get(text: string): T | undefined {
      const value = entries.get(text)
      if (value !== undefined) {
        entries.delete(text)
        entries.set(text, value)
      }
      return value
    },
    set(text: string, value: T) {
      if (text.length > limit) return
      if (entries.delete(text)) size -= text.length
      entries.set(text, value)
      size += text.length
      for (const oldest of entries.keys()) {
        if (size <= limit) break
        entries.delete(oldest)
        size -= oldest.length
      }
    }
  }
}
