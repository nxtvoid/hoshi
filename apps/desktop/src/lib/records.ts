/** The record without these keys; the same record when it has none of them. */
export function withoutKeys<T>(
  record: Record<string, T>,
  keys: readonly string[]
): Record<string, T> {
  if (!keys.some((key) => Object.hasOwn(record, key))) return record
  const next = { ...record }
  for (const key of keys) delete next[key]
  return next
}
