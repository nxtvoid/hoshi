import { create } from 'zustand'
import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { Thread } from '@/bindings/Thread'
import type { Command } from '@/bindings/Command'
import type { Project } from '@/bindings/Project'
import type { Snapshot } from '@/bindings/Snapshot'
import type { PullRequest } from '@/bindings/PullRequest'
import type { ThreadWorkspace } from '@/bindings/ThreadWorkspace'
import {
  type AccessChoice,
  type ModelChoice,
  useComposerStore
} from './composer-store'
import { confirm } from './dialog-store'
import { startupSelection } from './startup'
import { useSidebarStore } from './sidebar-store'
import { loadedSettings } from './settings-store'
import { useLastSelectionStore } from './last-selection-store'
import { useTranscriptStore } from './transcript-store'
import { usePullRequestStore } from './pull-request-store'
import { useDraftStore } from './draft-store'
import { useUsageStore } from './usage-store'
import { useProviderUpdateStore } from './provider-update-store'
import { watchAppUpdates } from './app-update-store'
import type { StreamDelta } from '@/bindings/StreamDelta'
import type { ProviderVersion } from '@/bindings/ProviderVersion'
import type { UsageLimits } from '@/bindings/UsageLimits'
import type { ContextUsage } from '@/bindings/ContextUsage'
import type { CommandError } from '@/bindings/CommandError'
import type { MessageAttachment } from '@/bindings/MessageAttachment'
import type { EventEnvelope } from '@/bindings/EventEnvelope'
import type { ThreadPullRequest } from '@/bindings/ThreadPullRequest'
import type { ThreadContextUsage } from '@/bindings/ThreadContextUsage'
import type { ThreadPullRequestDetail } from '@/bindings/ThreadPullRequestDetail'
import { inChat, scopeProjectId } from './scope'
import { applyEvent, folderName, samePath } from './orchestration'

type AppState = {
  ready: boolean
  sequence: number
  projects: Project[]
  threads: Thread[]
  /** The last known pull request of each thread that has one, by thread id. */
  pullRequests: Record<string, PullRequest>
  /** How full each thread's context was after its last model call, by thread id. */
  contextUsages: Record<string, ContextUsage>
  selectedProjectId: string | null
  selectedThreadId: string | null
  sidebarOpen: boolean
  search: string
  selectProject: (projectId: string) => void
  /** Opens a thread, switching to its project. */
  selectThread: (threadId: string) => void
  /** Leaves the open thread for an empty composer; the thread is created by its first message. */
  openDraft: () => void
  toggleSidebar: () => void
  setSearch: (search: string) => void

  /** Adds the folder at `path`, or selects the project already there. Resolves to why it couldn't. */
  addProjectAt: (path: string) => Promise<CommandError | null>
  removeProject: (projectId: string) => Promise<void>
  /**
   * Starts a thread where the main view works, or in `projectId` when given (`null` for a
   * chat); in the composer's workspace unless one is given.
   */
  createThread: (options?: {
    projectId?: string | null
    workspace?: ThreadWorkspace
    title?: string
  }) => Promise<string | null>
  /**
   * Opens the project's thread on the pull request, or starts one in a worktree on its
   * branch. Resolves to why it couldn't.
   */
  startThreadFromPullRequest: (
    pullRequest: PullRequest
  ) => Promise<CommandError | null>
  deleteThread: (threadId: string) => Promise<void>
  /** Names the thread again from its conversation; the new title arrives as an event. */
  regenerateTitle: (threadId: string) => Promise<void>
  archiveThread: (threadId: string, archived: boolean) => Promise<void>
  pinThread: (threadId: string, pinned: boolean) => Promise<void>
  /** Sends to the selected thread, creating one first when none is selected. */
  sendMessage: (
    text: string,
    attachments?: MessageAttachment[]
  ) => Promise<boolean>
  interruptTurn: () => Promise<void>
  /** Sets the selected thread's model and remembers it for new threads. */
  setModel: (choice: ModelChoice) => Promise<void>
  setAccess: (choice: AccessChoice) => Promise<void>
  /** The last command that failed from the composer, shown under it. */
  composerError: CommandError | null
  clearComposerError: () => void
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
  pullRequests: {},
  contextUsages: {},
  selectedProjectId: null,
  selectedThreadId: null,
  sidebarOpen: true,
  search: '',
  composerError: null,
  clearComposerError: () => set({ composerError: null }),
  selectProject: (projectId) => {
    set({ selectedProjectId: projectId, selectedThreadId: null, search: '' })
    void useTranscriptStore.getState().load(null)
  },
  selectThread: (threadId) => {
    if (get().selectedThreadId === threadId) return
    const projectId = get().threads.find(
      (thread) => thread.id === threadId
    )?.projectId
    set({
      selectedThreadId: threadId,
      ...(projectId ? { selectedProjectId: projectId } : {})
    })
    void useTranscriptStore.getState().load(threadId)
  },
  openDraft: () => {
    set({ selectedThreadId: null, search: '', composerError: null })
    void useTranscriptStore.getState().load(null)
  },
  toggleSidebar: () => set((state) => ({ sidebarOpen: !state.sidebarOpen })),
  setSearch: (search) => set({ search }),

  addProjectAt: async (path) => {
    const existing = get().projects.find((project) =>
      samePath(project.workspaceRoot, path)
    )
    if (existing) {
      get().selectProject(existing.id)
      return null
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
      return null
    } catch (error) {
      const commandError = error as CommandError
      if (commandError.code === 'duplicateWorkspaceRoot') {
        get().selectProject(commandError.projectId)
        return null
      }
      return commandError
    }
  },
  removeProject: async (projectId) => {
    const project = get().projects.find(
      (candidate) => candidate.id === projectId
    )
    if (!project) return
    const confirmed = await confirm({
      title: m.remove_project(),
      description: m.remove_project_confirm({ title: project.title }),
      confirmLabel: m.remove_project(),
      tone: 'destructive'
    })
    if (!confirmed) return
    await dispatch({ type: 'projectRemove', projectId }).catch(reportError)
  },
  createThread: async (options = {}) => {
    const projectId =
      options.projectId === undefined
        ? scopeProjectId(get(), useSidebarStore.getState().view)
        : options.projectId
    const chat =
      options.projectId === null ||
      (options.projectId === undefined &&
        inChat(get(), useSidebarStore.getState().view))
    if (!projectId && !chat) return null
    const threadId = crypto.randomUUID()
    const {
      model,
      effort,
      fastMode,
      ultracode,
      accessMode,
      planMode,
      workspace
    } = useComposerStore.getState()
    try {
      await dispatch({
        type: 'threadCreate',
        threadId,
        projectId: chat ? null : projectId,
        title: options.title ?? '',
        workspace: chat
          ? { type: 'local' }
          : (options.workspace ?? { type: workspace }),
        model,
        effort,
        fastMode,
        ultracode,
        accessMode,
        planMode
      })
      set({ search: '', composerError: null })
      get().selectThread(threadId)
      return threadId
    } catch (error) {
      reportError(error)
      set({ composerError: error as CommandError })
      return null
    }
  },
  startThreadFromPullRequest: async (pullRequest) => {
    const { threads, pullRequests, selectedProjectId } = get()
    const existing = threads.find(
      (thread) =>
        thread.projectId === selectedProjectId &&
        !thread.archived &&
        (pullRequests[thread.id]?.number === pullRequest.number ||
          thread.branch === pullRequest.headBranch)
    )
    if (existing) {
      get().selectThread(existing.id)
      return null
    }
    const threadId = await get().createThread({
      workspace: { type: 'branchWorktree', branch: pullRequest.headBranch },
      title: pullRequest.title
    })
    if (threadId) return null
    const error = get().composerError
    set({ composerError: null })
    return error ?? { code: 'internal', detail: '' }
  },
  regenerateTitle: async (threadId) => {
    await invoke('regenerate_thread_title', { threadId }).catch(reportError)
  },
  deleteThread: async (threadId) => {
    const thread = get().threads.find((candidate) => candidate.id === threadId)
    if (!thread) return
    const confirmed = await confirm({
      title: m.delete_thread(),
      description: m.delete_thread_confirm({
        title: thread.title || m.untitled_thread()
      }),
      confirmLabel: m.delete_thread(),
      tone: 'destructive'
    })
    if (!confirmed) return
    await dispatch({ type: 'threadDelete', threadId }).catch(reportError)
  },
  archiveThread: async (threadId, archived) => {
    await dispatch({ type: 'threadArchive', threadId, archived }).catch(
      reportError
    )
  },
  pinThread: async (threadId, pinned) => {
    await dispatch({ type: 'threadPin', threadId, pinned }).catch(reportError)
  },
  sendMessage: async (text, attachments = []) => {
    const threadId = get().selectedThreadId ?? (await get().createThread())
    if (!threadId) return false
    try {
      await dispatch({
        type: 'messageSend',
        threadId,
        messageId: crypto.randomUUID(),
        text,
        attachments
      })
      set({ composerError: null })
      return true
    } catch (error) {
      reportError(error)
      set({ composerError: error as CommandError })
      return false
    }
  },
  setModel: async (choice) => {
    useComposerStore.getState().setModel(choice)
    const threadId = get().selectedThreadId
    if (!threadId) return
    await dispatch({ type: 'threadModelSet', threadId, ...choice }).catch(
      reportError
    )
  },
  setAccess: async (choice) => {
    useComposerStore.getState().setAccess(choice)
    const threadId = get().selectedThreadId
    if (!threadId) return
    await dispatch({ type: 'threadAccessSet', threadId, ...choice }).catch(
      reportError
    )
  },
  interruptTurn: async () => {
    const threadId = get().selectedThreadId
    if (!threadId) return
    await dispatch({ type: 'turnInterrupt', threadId }).catch(reportError)
  }
}))

function receive({ sequence, event }: EventEnvelope) {
  if (sequence <= useAppStore.getState().sequence) return
  useTranscriptStore.getState().applyEvent(event)
  if (event.type === 'threadDeleted') {
    useDraftStore.getState().save(event.threadId, null)
  }
  useAppStore.setState((state) => {
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
  const { selectedThreadId } = useAppStore.getState()
  if (selectedThreadId !== useTranscriptStore.getState().threadId) {
    void useTranscriptStore.getState().load(selectedThreadId)
  }
}

function receivePullRequest({ threadId, pullRequest }: ThreadPullRequest) {
  useAppStore.setState((state) => {
    const pullRequests = { ...state.pullRequests }
    if (pullRequest) pullRequests[threadId] = pullRequest
    else delete pullRequests[threadId]
    return { pullRequests }
  })
}

function receiveContextUsage({ threadId, usage }: ThreadContextUsage) {
  useAppStore.setState((state) => ({
    contextUsages: { ...state.contextUsages, [threadId]: usage }
  }))
}

async function startSync() {
  let buffered: EventEnvelope[] | null = []
  let bufferedPullRequests: ThreadPullRequest[] | null = []
  let bufferedContextUsages: ThreadContextUsage[] | null = []
  const unlisten = await listen<EventEnvelope>('hoshi://event', (message) => {
    if (buffered) buffered.push(message.payload)
    else receive(message.payload)
  })
  const unlistenPullRequests = await listen<ThreadPullRequest>(
    'hoshi://pull-request',
    (message) => {
      if (bufferedPullRequests) bufferedPullRequests.push(message.payload)
      else receivePullRequest(message.payload)
    }
  )
  const unlistenPullRequestDetails = await listen<ThreadPullRequestDetail>(
    'hoshi://pull-request-detail',
    (message) => usePullRequestStore.getState().receive(message.payload)
  )
  const unlistenContextUsages = await listen<ThreadContextUsage>(
    'hoshi://context-usage',
    (message) => {
      if (bufferedContextUsages) bufferedContextUsages.push(message.payload)
      else receiveContextUsage(message.payload)
    }
  )
  const unlistenUsageLimits = await listen<UsageLimits>(
    'hoshi://usage-limits',
    (message) => useUsageStore.getState().receive(message.payload)
  )
  const unlistenProviderVersion = await listen<ProviderVersion | null>(
    'hoshi://provider-version',
    (message) => useProviderUpdateStore.getState().receive(message.payload)
  )
  const stopAppUpdates = watchAppUpdates()
  const unlistenStream = await listen<StreamDelta>(
    'hoshi://stream',
    (message) => useTranscriptStore.getState().applyDelta(message.payload)
  )
  import.meta.hot?.dispose(() => {
    unlisten()
    unlistenStream()
    unlistenPullRequests()
    unlistenPullRequestDetails()
    unlistenContextUsages()
    unlistenUsageLimits()
    unlistenProviderVersion()
    stopAppUpdates()
  })
  const snapshot = await invoke<Snapshot>('get_snapshot')
  const settings = await loadedSettings()
  const start = startupSelection(
    settings?.general.startupView ?? 'inbox',
    useLastSelectionStore.getState(),
    snapshot.projects,
    snapshot.threads
  )
  useSidebarStore.getState().setView(start.sidebarView)
  useAppStore.setState({
    ready: true,
    sequence: snapshot.sequence,
    projects: snapshot.projects,
    threads: snapshot.threads,
    pullRequests: Object.fromEntries(
      snapshot.pullRequests.flatMap(({ threadId, pullRequest }) =>
        pullRequest ? [[threadId, pullRequest]] : []
      )
    ),
    contextUsages: Object.fromEntries(
      snapshot.contextUsages.map(({ threadId, usage }) => [threadId, usage])
    ),
    selectedProjectId: start.projectId,
    selectedThreadId: start.threadId
  })
  if (start.threadId) void useTranscriptStore.getState().load(start.threadId)
  void useUsageStore.getState().load()
  void useProviderUpdateStore.getState().load()
  for (const envelope of buffered) receive(envelope)
  buffered = null
  for (const update of bufferedPullRequests) receivePullRequest(update)
  bufferedPullRequests = null
  for (const update of bufferedContextUsages) receiveContextUsage(update)
  bufferedContextUsages = null
}

useAppStore.subscribe((state, previous) => {
  if (!state.ready) return
  if (
    state.selectedProjectId !== previous.selectedProjectId ||
    state.selectedThreadId !== previous.selectedThreadId
  ) {
    useLastSelectionStore.setState({
      projectId: state.selectedProjectId,
      threadId: state.selectedThreadId
    })
  }
})

startSync().catch((error) => console.error('hoshi: sync failed', error))
