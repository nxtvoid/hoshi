import { create } from 'zustand'
import type { Thread } from '@/bindings/Thread'
import type { Project } from '@/bindings/Project'
import { fixtureProjects, fixtureThreads } from './fixtures'

type AppState = {
  projects: Project[]
  threads: Thread[]
  selectedProjectId: string | null
  selectedThreadId: string | null
  sidebarOpen: boolean
  search: string
  selectProject: (projectId: string) => void
  selectThread: (threadId: string) => void
  toggleSidebar: () => void
  setSearch: (search: string) => void
  createThread: () => void
}

export const useAppStore = create<AppState>()((set, get) => ({
  projects: fixtureProjects,
  threads: fixtureThreads,
  selectedProjectId: fixtureProjects[0]?.id ?? null,
  selectedThreadId: null,
  sidebarOpen: true,
  search: '',
  selectProject: (projectId) =>
    set({ selectedProjectId: projectId, selectedThreadId: null, search: '' }),
  selectThread: (threadId) => set({ selectedThreadId: threadId }),
  toggleSidebar: () => set((state) => ({ sidebarOpen: !state.sidebarOpen })),
  setSearch: (search) => set({ search }),
  createThread: () => {
    const projectId = get().selectedProjectId
    if (!projectId) return
    const thread: Thread = {
      id: crypto.randomUUID(),
      projectId,
      title: '',
      status: 'idle',
      branch: null,
      updatedAt: new Date().toISOString()
    }
    set((state) => ({
      threads: [thread, ...state.threads],
      selectedThreadId: thread.id,
      search: ''
    }))
  }
}))
