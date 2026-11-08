import type { Thread } from '@/bindings/Thread'
import type { PullRequest } from '@/bindings/PullRequest'

export type SectionId =
  | 'pinned'
  | 'attention'
  | 'active'
  | 'review'
  | 'inactive'
  | 'done'
  /** Inbox only: turns that finished while the user looked elsewhere. */
  | 'completed'

export type ThreadSection = { id: SectionId; threads: Thread[] }

export type ThreadSort = 'activity' | 'created'

/** Which threads the sidebar lists: one project's, the chats outside any project, or what needs the user across all of them. */
export type SidebarView = 'project' | 'chats' | 'inbox'

export type ThreadFilter = 'all' | 'worktrees' | 'pullRequests'

export type SidebarQuery = {
  view: SidebarView
  projectId: string | null
  search: string
  filter: ThreadFilter
  sort: ThreadSort
  /** Defaults to `INACTIVE_AFTER_MS`. */
  inactiveAfterMs?: number
}

const INBOX_SECTIONS: SectionId[] = [
  'attention',
  'active',
  'completed',
  'review'
]

const SECTION_ORDER: SectionId[] = [
  'pinned',
  'attention',
  'active',
  'review',
  'inactive',
  'done'
]

const DAY_MS = 24 * 60 * 60 * 1000

/** Idle threads untouched for this long stop counting as active, unless the settings say otherwise. */
export const INACTIVE_AFTER_MS = 3 * DAY_MS

export function inactiveAfterMs(days: number | undefined): number {
  return days && days > 0 ? days * DAY_MS : INACTIVE_AFTER_MS
}

/** An open pull request with failing checks or unresolved review threads waits on the user. */
export function pullRequestNeedsAttention(pullRequest: PullRequest): boolean {
  const open = pullRequest.state === 'open' || pullRequest.state === 'draft'
  return (
    open &&
    (pullRequest.checks === 'failing' || pullRequest.unresolvedComments > 0)
  )
}

export function sectionOf(
  thread: Thread,
  pullRequest: PullRequest | undefined,
  now: number,
  inactiveAfter = INACTIVE_AFTER_MS
): SectionId {
  if (thread.archived) return 'done'
  if (thread.pinned) return 'pinned'
  if (thread.status === 'waitingForInput' || thread.status === 'error') {
    return 'attention'
  }
  if (thread.status === 'working') return 'active'
  if (pullRequest) {
    if (pullRequestNeedsAttention(pullRequest)) return 'attention'
    if (pullRequest.state === 'merged' || pullRequest.state === 'closed') {
      return 'done'
    }
    return 'review'
  }
  const idleFor = now - Date.parse(thread.updatedAt)
  return idleFor > inactiveAfter ? 'inactive' : 'active'
}

/** Splits already sorted threads into sections, keeping their order. Empty sections are left out. */
export function groupThreads(
  threads: Thread[],
  pullRequests: Record<string, PullRequest>,
  now: number,
  inactiveAfter = INACTIVE_AFTER_MS
): ThreadSection[] {
  const bySection = new Map<SectionId, Thread[]>()
  for (const thread of threads) {
    const id = sectionOf(thread, pullRequests[thread.id], now, inactiveAfter)
    const section = bySection.get(id)
    if (section) section.push(thread)
    else bySection.set(id, [thread])
  }
  return SECTION_ORDER.flatMap((id) => {
    const sectionThreads = bySection.get(id)
    return sectionThreads ? [{ id, threads: sectionThreads }] : []
  })
}

/** Every word of the search has to appear in the title, the branch or the pull request (`#12`). */
export function matchesSearch(
  thread: Thread,
  pullRequest: PullRequest | undefined,
  search: string
): boolean {
  const words = search.trim().toLowerCase().split(/\s+/).filter(Boolean)
  if (words.length === 0) return true
  const fields = [thread.title, thread.branch ?? '']
  if (pullRequest) fields.push(`#${pullRequest.number}`, pullRequest.title)
  const haystack = fields.join(' ').toLowerCase()
  return words.every((word) => haystack.includes(word))
}

export function matchesFilter(
  thread: Thread,
  pullRequest: PullRequest | undefined,
  filter: ThreadFilter
): boolean {
  switch (filter) {
    case 'all':
      return true
    case 'worktrees':
      return thread.worktreePath !== null
    case 'pullRequests':
      return pullRequest !== undefined
  }
}

function inView(thread: Thread, query: SidebarQuery) {
  switch (query.view) {
    case 'inbox':
      return true
    case 'chats':
      return thread.projectId === null
    case 'project':
      return thread.projectId !== null && thread.projectId === query.projectId
  }
}

const ESCALATION: Partial<Record<Thread['status'], number>> = {
  working: 1,
  waitingForInput: 2
}

/**
 * The threads the user started. Tasks their agents delegated aren't listed: a task that
 * works, or waits for the user, shows on the thread that delegated it.
 */
export function withDelegatedWork(threads: Thread[]): Thread[] {
  if (!threads.some((thread) => thread.parentThreadId !== null)) return threads
  const ids = new Set(threads.map((thread) => thread.id))
  // A task whose thread is gone is the user's to see.
  const delegated = (thread: Thread) =>
    thread.parentThreadId !== null && ids.has(thread.parentThreadId)
  const busiest = new Map<string, Thread['status']>()
  for (const thread of threads) {
    const { parentThreadId, status } = thread
    const rank = ESCALATION[status]
    if (parentThreadId === null || !delegated(thread) || rank === undefined)
      continue
    const current = busiest.get(parentThreadId)
    if (!current || rank > (ESCALATION[current] ?? 0)) {
      busiest.set(parentThreadId, status)
    }
  }
  return threads
    .filter((thread) => !delegated(thread))
    .map((thread) => {
      const delegated = busiest.get(thread.id)
      const own =
        ESCALATION[thread.status] ?? (thread.status === 'error' ? 3 : 0)
      return delegated && (ESCALATION[delegated] ?? 0) > own
        ? { ...thread, status: delegated }
        : thread
    })
}

/** The sections the sidebar shows for a view, search and filter. */
export function sidebarSections(
  threads: Thread[],
  pullRequests: Record<string, PullRequest>,
  query: SidebarQuery,
  now: number,
  unseen: Record<string, true> = {}
): ThreadSection[] {
  const key = query.sort === 'created' ? 'createdAt' : 'updatedAt'
  const visible = withDelegatedWork(threads)
    .filter(
      (thread) =>
        inView(thread, query) &&
        matchesSearch(thread, pullRequests[thread.id], query.search) &&
        matchesFilter(thread, pullRequests[thread.id], query.filter)
    )
    .sort((a, b) => b[key].localeCompare(a[key]))
  return query.view === 'inbox'
    ? inboxSections(visible, pullRequests, now, unseen)
    : groupThreads(visible, pullRequests, now, query.inactiveAfterMs)
}

/**
 * What waits on the user in every project: needs attention, working now, finished but not
 * seen yet, and in review. Pins don't apply here, and idle threads stay out.
 */
export function inboxSections(
  threads: Thread[],
  pullRequests: Record<string, PullRequest>,
  now: number,
  unseen: Record<string, true> = {}
): ThreadSection[] {
  const completed = threads.filter(
    (thread) =>
      unseen[thread.id] && thread.status === 'idle' && !thread.archived
  )
  const rest = threads
    .filter((thread) => !completed.includes(thread))
    .map((thread) => (thread.pinned ? { ...thread, pinned: false } : thread))
  const original = new Map(threads.map((thread) => [thread.id, thread]))
  const grouped = groupThreads(rest, pullRequests, now).flatMap((section) => {
    if (!INBOX_SECTIONS.includes(section.id)) return []
    const kept = section.threads
      .filter(
        (thread) => section.id !== 'active' || thread.status === 'working'
      )
      .map((thread) => original.get(thread.id) ?? thread)
    return kept.length > 0 ? [{ ...section, threads: kept }] : []
  })
  if (completed.length > 0)
    grouped.push({ id: 'completed', threads: completed })
  return INBOX_SECTIONS.flatMap((id) =>
    grouped.filter((section) => section.id === id)
  )
}

/** What the inbox counts: threads that need the user, and turns they haven't seen. */
export function inboxCount(sections: ThreadSection[]): number {
  return sections
    .filter(
      (section) => section.id === 'attention' || section.id === 'completed'
    )
    .reduce((total, section) => total + section.threads.length, 0)
}

/** Threads in the order the sidebar shows them; a folded section only shows the selected one. */
export function visibleOrder(
  sections: ThreadSection[],
  isOpen: (section: SectionId) => boolean,
  selectedThreadId: string | null
): string[] {
  return sections.flatMap((section) =>
    section.threads
      .filter((thread) => isOpen(section.id) || thread.id === selectedThreadId)
      .map((thread) => thread.id)
  )
}

/** The thread `step` places away from the current one, wrapping around; the first when none is current. */
export function stepThread(
  order: string[],
  current: string | null,
  step: 1 | -1
): string | null {
  if (order.length === 0) return null
  const index = current === null ? -1 : order.indexOf(current)
  if (index === -1)
    return step === 1 ? (order[0] ?? null) : (order.at(-1) ?? null)
  return order[(index + step + order.length) % order.length] ?? null
}
