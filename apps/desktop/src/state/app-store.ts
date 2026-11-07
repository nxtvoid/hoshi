import { create } from 'zustand'
import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { Thread } from '@/bindings/Thread'
import type { Command } from '@/bindings/Command'
import type { Project } from '@/bindings/Project'
import type { Snapshot } from '@/bindings/Snapshot'
import { ask, open } from '@tauri-apps/plugin-dialog'
import type { CommandError } from '@/bindings/CommandError'
import type { EventEnvelope } from '@/bindings/EventEnvelope'
import { applyEvent, folderName, samePath } from './orchestration'

type AppState = {
  ready: boolean
  sequence: number
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
  addProject: () => Promise<void>
  removeProject: (projectId: string) => Promise<void>
  createThread: () => Promise<void>
  deleteThread: (threadId: string) => Promise<void>
}

const dispatch = (command: Command) => invoke<void>('dispatch', { command })

function reportError(error: unknown) {
  console.error('hoshi: command failed', error as CommandError)
}

export const useAppStore = create<AppState>()((set, get) => ({
  ready: false,
  sequence: 0,
  projects: [],
  threads: [],
  selectedProjectId: null,
  selectedThreadId: null,
  sidebarOpen: true,
  search: '',
  selectProject: (projectId) =>
    set({ selectedProjectId: projectId, selectedThreadId: null, search: '' }),
  selectThread: (threadId) => set({ selectedThreadId: threadId }),
  toggleSidebar: () => set((state) => ({ sidebarOpen: !state.sidebarOpen })),
  setSearch: (search) => set({ search }),
  addProject: async () => {
    const path = await open({ directory: true, multiple: false })
    if (!path) return
    const existing = get().projects.find((project) =>
      samePath(project.workspaceRoot, path)
    )
    if (existing) {
      get().selectProject(existing.id)
      return
    }
    const projectId = crypto.randomUUID()
    try {
      await dispatch({
        type: 'projectCreate',
        projectId,
        title: folderName(path),
        workspaceRoot: path
      })
      get().selectProject(projectId)
    } catch (error) {
      const commandError = error as CommandError
      if (commandError.code === 'duplicateWorkspaceRoot') {
        get().selectProject(commandError.projectId)
        return
      }
      reportError(error)
    }
  },
  removeProject: async (projectId) => {
    const project = get().projects.find(
      (candidate) => candidate.id === projectId
    )
    if (!project) return
    const confirmed = await ask(
      m.remove_project_confirm({ title: project.title }),
      { kind: 'warning', okLabel: m.remove_project() }
    )
    if (!confirmed) return
    await dispatch({ type: 'projectRemove', projectId }).catch(reportError)
  },
  createThread: async () => {
    const projectId = get().selectedProjectId
    if (!projectId) return
    const threadId = crypto.randomUUID()
    try {
      await dispatch({ type: 'threadCreate', threadId, projectId, title: '' })
      set({ selectedThreadId: threadId, search: '' })
    } catch (error) {
      reportError(error)
    }
  },
  deleteThread: async (threadId) => {
    const thread = get().threads.find((candidate) => candidate.id === threadId)
    if (!thread) return
    const confirmed = await ask(
      m.delete_thread_confirm({
        title: thread.title || m.untitled_thread()
      }),
      { kind: 'warning', okLabel: m.delete_thread() }
    )
    if (!confirmed) return
    await dispatch({ type: 'threadDelete', threadId }).catch(reportError)
  }
}))

function receive({ sequence, event }: EventEnvelope) {
  useAppStore.setState((state) => {
    if (sequence <= state.sequence) return state
    const next = applyEvent(state, event)
    const selectedProjectId = next.projects.some(
      (project) => project.id === state.selectedProjectId
    )
      ? state.selectedProjectId
      : (next.projects[0]?.id ?? null)
    const selectedThreadId = next.threads.some(
      (thread) => thread.id === state.selectedThreadId
    )
      ? state.selectedThreadId
      : null
    return { ...next, sequence, selectedProjectId, selectedThreadId }
  })
}

async function startSync() {
  let buffered: EventEnvelope[] | null = []
  const unlisten = await listen<EventEnvelope>('hoshi://event', (message) => {
    if (buffered) buffered.push(message.payload)
    else receive(message.payload)
  })
  import.meta.hot?.dispose(unlisten)
  const snapshot = await invoke<Snapshot>('get_snapshot')
  useAppStore.setState({
    ready: true,
    sequence: snapshot.sequence,
    projects: snapshot.projects,
    threads: snapshot.threads,
    selectedProjectId: snapshot.projects[0]?.id ?? null
  })
  for (const envelope of buffered) receive(envelope)
  buffered = null
}

startSync().catch((error) => console.error('hoshi: sync failed', error))
