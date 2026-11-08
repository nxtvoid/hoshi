type Searchable = { name: string; description: string; aliases: string[] }

const BOUNDARY = /[-_:]/

/** Lower is better; `null` when the command does not match. */
function score(command: Searchable, query: string): number | null {
  const name = command.name.toLowerCase()
  const aliases = command.aliases.map((alias) => alias.toLowerCase())
  if (name === query) return 0
  if (aliases.includes(query)) return 1
  if (name.startsWith(query)) return 2
  if (aliases.some((alias) => alias.startsWith(query))) return 3
  const boundary = name.indexOf(query)
  if (boundary > 0 && BOUNDARY.test(name[boundary - 1] ?? '')) return 4
  if (boundary > 0) return 5
  if (command.description.toLowerCase().includes(query)) return 20
  return isSubsequence(name, query) ? 50 : null
}

function isSubsequence(text: string, query: string) {
  let next = 0
  for (const char of text) {
    if (char === query[next]) next++
    if (next === query.length) return true
  }
  return false
}

/** The items whose command matches `query`, best first; all of them, in order, when it is empty. */
export function rankCommands<T>(
  items: readonly T[],
  query: string,
  commandOf: (item: T) => Searchable
): T[] {
  const needle = query.trim().toLowerCase()
  if (needle === '') return [...items]
  return items
    .map((item, index) => ({
      item,
      index,
      score: score(commandOf(item), needle)
    }))
    .filter(
      (entry): entry is typeof entry & { score: number } => entry.score !== null
    )
    .sort((a, b) => a.score - b.score || a.index - b.index)
    .map((entry) => entry.item)
}
