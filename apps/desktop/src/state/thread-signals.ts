import type { Thread } from '@/bindings/Thread'

/** The strongest thing a project's threads have to tell, for its rail button. */
export type ProjectSignal = 'error' | 'waiting' | 'done' | 'working'

/** Threads whose turn ended between two snapshots, without failing or asking for input. */
export function finishedTurns(previous: Thread[], next: Thread[]): string[] {
  const wasWorking = new Set(
    previous
      .filter((thread) => thread.status === 'working')
      .map((thread) => thread.id)
  )
  return next
    .filter((thread) => wasWorking.has(thread.id) && thread.status === 'idle')
    .map((thread) => thread.id)
}

export type ThreadNotice = {
  threadId: string
  kind: 'finished' | 'failed' | 'waiting'
}

/** What changed between two snapshots that's worth telling the user: a turn that ended, failed or asks something. */
export function threadNotices(
  previous: Thread[],
  next: Thread[]
): ThreadNotice[] {
  const before = new Map(previous.map((thread) => [thread.id, thread.status]))
  return next.flatMap((thread): ThreadNotice[] => {
    const was = before.get(thread.id)
    if (was === undefined || was === thread.status) return []
    if (thread.status === 'error')
      return [{ threadId: thread.id, kind: 'failed' }]
    if (thread.status === 'waitingForInput')
      return [{ threadId: thread.id, kind: 'waiting' }]
    if (was === 'working' && thread.status === 'idle')
      return [{ threadId: thread.id, kind: 'finished' }]
    return []
  })
}

/**
 * A failure first, then a question, then a finished turn not seen yet, then work going on.
 * A `null` project stands for the chats.
 */
export function projectSignal(
  threads: Thread[],
  projectId: string | null,
  unseen: Record<string, true>
): ProjectSignal | null {
  let signal: ProjectSignal | null = null
  const rank: Record<ProjectSignal, number> = {
    error: 4,
    waiting: 3,
    done: 2,
    working: 1
  }
  for (const thread of threads) {
    if (thread.projectId !== projectId || thread.archived) continue
    const own: ProjectSignal | null =
      thread.status === 'error'
        ? 'error'
        : thread.status === 'waitingForInput'
          ? 'waiting'
          : thread.status === 'working'
            ? 'working'
            : unseen[thread.id]
              ? 'done'
              : null
    if (own && (!signal || rank[own] > rank[signal])) signal = own
  }
  return signal
}
