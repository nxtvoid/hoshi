import type { Thread } from '@/bindings/Thread'
import { newestFirst } from './thread-sections'
import type { Project } from '@/bindings/Project'
import type { PullRequest } from '@/bindings/PullRequest'

export type PaletteAction =
  | 'newThread'
  | 'newThreadFromPullRequest'
  | 'addProject'
  | 'newNote'
  | 'notes'
  | 'inbox'
  | 'settings'
  | 'usage'

export const PALETTE_ACTIONS: PaletteAction[] = [
  'newThread',
  'newThreadFromPullRequest',
  'addProject',
  'newNote',
  'notes',
  'inbox',
  'settings',
  'usage'
]

const THREAD_LIMIT = 8
const RECENT_LIMIT = 6
const PROJECT_LIMIT = 5

export type PaletteResults = {
  threads: Thread[]
  projects: Project[]
  actions: PaletteAction[]
}

export type PaletteSource = {
  threads: Thread[]
  projects: Project[]
  pullRequests: Record<string, PullRequest>
  actionLabels: Record<PaletteAction, string>
}

/** 3 when the text starts with the word, 2 when one of its words does, 1 when it only contains it. */
export function scoreField(text: string, word: string): number {
  const lower = text.toLowerCase()
  const at = lower.indexOf(word)
  if (at === -1) return 0
  if (at === 0) return 3
  return /[\s/_.#-]/.test(lower[at - 1] ?? '') ? 2 : 1
}

/** Every word has to match one of the fields; the best field of each word adds up. */
function score(fields: string[], words: string[]): number {
  let total = 0
  for (const word of words) {
    const best = Math.max(0, ...fields.map((field) => scoreField(field, word)))
    if (best === 0) return 0
    total += best
  }
  return total
}

function ranked<T>(
  items: T[],
  fields: (item: T) => string[],
  words: string[],
  tieBreak: (a: T, b: T) => number
): T[] {
  return items
    .map((item) => ({ item, score: score(fields(item), words) }))
    .filter((entry) => entry.score > 0)
    .sort((a, b) => b.score - a.score || tieBreak(a.item, b.item))
    .map((entry) => entry.item)
}

const newest = (a: Thread, b: Thread) =>
  Number(a.archived) - Number(b.archived) ||
  newestFirst(a.updatedAt, b.updatedAt)

/**
 * Threads match their title, branch, pull request (`#12` or its title) and project; with no
 * query the palette shows the latest threads and every action.
 */
export function searchPalette(
  query: string,
  source: PaletteSource
): PaletteResults {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean)
  if (words.length === 0) {
    return {
      threads: source.threads
        .filter((thread) => !thread.archived)
        .sort(newest)
        .slice(0, RECENT_LIMIT),
      projects: [],
      actions: PALETTE_ACTIONS
    }
  }
  const projectTitle = new Map(
    source.projects.map((project) => [project.id, project.title])
  )
  const threadFields = (thread: Thread) => {
    const pullRequest = source.pullRequests[thread.id]
    return [
      thread.title,
      thread.branch ?? '',
      (thread.projectId && projectTitle.get(thread.projectId)) || '',
      ...(pullRequest ? [`#${pullRequest.number}`, pullRequest.title] : [])
    ]
  }
  return {
    threads: ranked(source.threads, threadFields, words, newest).slice(
      0,
      THREAD_LIMIT
    ),
    projects: ranked(
      source.projects,
      (project) => [project.title, project.workspaceRoot],
      words,
      (a, b) => a.title.localeCompare(b.title)
    ).slice(0, PROJECT_LIMIT),
    actions: ranked(
      PALETTE_ACTIONS,
      (action) => [source.actionLabels[action]],
      words,
      () => 0
    )
  }
}
