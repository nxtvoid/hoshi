import type { Thread } from '@/bindings/Thread'

/** Whether two versions of a thread differ in nothing but `updatedAt`. */
function sameButUpdated(previous: Thread, next: Thread) {
  for (const key of Object.keys(next) as (keyof Thread)[]) {
    if (key !== 'updatedAt' && previous[key] !== next[key]) return false
  }
  return true
}

/**
 * The threads as lists show them: a working thread whose items only moved its `updatedAt`
 * keeps the version shown before, so it holds its place and nothing redraws on each item.
 * The same list comes back while no thread changed otherwise.
 */
export function quietThreads(
  previous: readonly Thread[] | null,
  threads: readonly Thread[]
): Thread[] {
  if (!previous) return [...threads]
  const shown = new Map(previous.map((thread) => [thread.id, thread]))
  let changed = previous.length !== threads.length
  const quiet = threads.map((thread, index) => {
    const before = shown.get(thread.id)
    const kept =
      before !== undefined &&
      (before === thread ||
        (thread.status === 'working' &&
          before.status === 'working' &&
          sameButUpdated(before, thread)))
    const next = kept ? before : thread
    if (next !== previous[index]) changed = true
    return next
  })
  return changed ? quiet : (previous as Thread[])
}

let last: { threads: readonly Thread[]; quiet: Thread[] } | null = null

/** `quietThreads` of the store's threads, for selectors; the same list while nothing moved. */
export function selectQuietThreads(state: { threads: readonly Thread[] }) {
  if (last?.threads === state.threads) return last.quiet
  const quiet = quietThreads(last?.quiet ?? null, state.threads)
  last = { threads: state.threads, quiet }
  return quiet
}
