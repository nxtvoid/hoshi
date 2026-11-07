import type { Thread } from '@/bindings/Thread'
import type { Project } from '@/bindings/Project'

const minutesAgo = (minutes: number) =>
  new Date(Date.now() - minutes * 60_000).toISOString()

export const fixtureProjects: Project[] = [
  { id: 'p-hoshi', name: 'hoshi', path: 'C:/projects/hoshi' },
  { id: 'p-portfolio', name: 'portfolio', path: 'C:/projects/portfolio' },
  { id: 'p-api', name: 'api-server', path: 'C:/projects/api-server' },
  { id: 'p-notes', name: 'notes', path: 'C:/projects/notes' }
]

export const fixtureThreads: Thread[] = [
  {
    id: 't-1',
    projectId: 'p-hoshi',
    title: 'Build the sidebar layout',
    status: 'working',
    branch: 'feat/sidebar',
    updatedAt: minutesAgo(2)
  },
  {
    id: 't-2',
    projectId: 'p-hoshi',
    title: 'Claude Code adapter',
    status: 'waitingForInput',
    branch: 'feat/claude-adapter',
    updatedAt: minutesAgo(18)
  },
  {
    id: 't-3',
    projectId: 'p-hoshi',
    title: 'Event store on SQLite',
    status: 'idle',
    branch: null,
    updatedAt: minutesAgo(60 * 5)
  },
  {
    id: 't-4',
    projectId: 'p-hoshi',
    title: 'Fix worktree branch detection',
    status: 'error',
    branch: 'fix/worktree-branch',
    updatedAt: minutesAgo(60 * 26)
  },
  {
    id: 't-5',
    projectId: 'p-portfolio',
    title: 'Redesign the landing page',
    status: 'idle',
    branch: 'main',
    updatedAt: minutesAgo(60 * 50)
  },
  {
    id: 't-6',
    projectId: 'p-api',
    title: 'Add rate limiting',
    status: 'idle',
    branch: 'feat/rate-limit',
    updatedAt: minutesAgo(60 * 24 * 9)
  }
]
