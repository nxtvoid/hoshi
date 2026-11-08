import type { Event } from '@/bindings/Event'
import type { Thread } from '@/bindings/Thread'
import type { Project } from '@/bindings/Project'
import type { ThreadItem } from '@/bindings/ThreadItem'

export type OrchestrationState = {
  projects: Project[]
  threads: Thread[]
}

/** Mirrors `ReadModel::apply` in hoshi-core for the sidebar's state. */
export function applyEvent(
  state: OrchestrationState,
  event: Event
): OrchestrationState {
  const updateThread = (threadId: string, update: Partial<Thread>) => ({
    ...state,
    threads: state.threads.map((thread) =>
      thread.id === threadId ? { ...thread, ...update } : thread
    )
  })
  switch (event.type) {
    case 'projectCreated':
      return { ...state, projects: [...state.projects, event.project] }
    case 'projectRemoved':
      return {
        projects: state.projects.filter(
          (project) => project.id !== event.projectId
        ),
        threads: state.threads.filter(
          (thread) => thread.projectId !== event.projectId
        )
      }
    case 'threadCreated':
      return { ...state, threads: [...state.threads, event.thread] }
    case 'threadDeleted':
      return {
        ...state,
        threads: state.threads.filter((thread) => thread.id !== event.threadId)
      }
    case 'threadTitleSet':
      return updateThread(event.threadId, { title: event.title })
    case 'threadBranchSet':
      return updateThread(event.threadId, { branch: event.branch })
    case 'threadWorktreeAttached':
      return updateThread(event.threadId, {
        worktreePath: event.worktreePath,
        branch: event.branch,
        worktreeReleased: false,
        worktreeAdopted: event.adopted
      })
    case 'threadWorktreeDetached':
      return updateThread(event.threadId, {
        worktreePath: null,
        branch: event.branch,
        worktreeReleased: false,
        worktreeAdopted: false
      })
    case 'threadWorktreeReleased':
      return updateThread(event.threadId, { worktreeReleased: true })
    case 'threadWorktreeRestored':
      return updateThread(event.threadId, { worktreeReleased: false })
    case 'threadCleanedUp':
      return updateThread(event.threadId, {
        worktreePath: null,
        branch: null,
        worktreeReleased: false,
        worktreeAdopted: false,
        sessionId: null
      })
    case 'threadArchivedSet':
      return updateThread(event.threadId, { archived: event.archived })
    case 'threadPinnedSet':
      return updateThread(event.threadId, { pinned: event.pinned })
    case 'threadModelSet':
      return updateThread(event.threadId, {
        model: event.model,
        effort: event.effort,
        fastMode: event.fastMode,
        ultracode: event.ultracode
      })
    case 'threadAccessSet':
      return updateThread(event.threadId, {
        accessMode: event.accessMode,
        planMode: event.planMode
      })
    case 'turnStarted':
      return updateThread(event.threadId, {
        status: 'working',
        activeTurnId: event.turnId,
        updatedAt: event.at
      })
    case 'sessionAttached':
      return updateThread(event.threadId, { sessionId: event.sessionId })
    case 'itemAdded':
      return updateThread(event.item.threadId, {
        updatedAt: event.item.createdAt
      })
    case 'toolCallCompleted':
    case 'toolCallInterrupted':
      return state
    case 'turnCompleted':
      return updateThread(event.threadId, {
        status: event.outcome.type === 'failed' ? 'error' : 'idle',
        activeTurnId: null,
        updatedAt: event.at
      })
    case 'threadStatusSet':
      return updateThread(event.threadId, { status: event.status })
    case 'threadDelegated':
      return updateThread(event.threadId, {
        parentThreadId: event.parentThreadId,
        ...(event.sharesWorktree ? { worktreeAdopted: true } : {})
      })
    case 'taskCancelled':
      return updateThread(event.threadId, { taskCancelled: true })
    case 'taskAnswered':
      return updateThread(event.threadId, { taskAnswered: true })
  }
}

/** Applies an event to one thread's transcript. Repeated items are replaced. */
export function applyItemEvent(
  threadId: string,
  items: ThreadItem[],
  event: Event
): ThreadItem[] {
  if (event.type === 'itemAdded' && event.item.threadId === threadId) {
    const index = items.findIndex((item) => item.id === event.item.id)
    if (index === -1) return [...items, event.item]
    return items.map((item, position) =>
      position === index ? event.item : item
    )
  }
  if (event.type === 'toolCallCompleted' && event.threadId === threadId) {
    return items.map((item) =>
      item.id === event.itemId && item.content.kind === 'toolCall'
        ? {
            ...item,
            content: {
              ...item.content,
              output: event.output,
              isError: event.isError,
              detail: event.detail
            }
          }
        : item
    )
  }
  if (event.type === 'toolCallInterrupted' && event.threadId === threadId) {
    return items.map((item) =>
      item.id === event.itemId && item.content.kind === 'toolCall'
        ? { ...item, content: { ...item.content, interrupted: true } }
        : item
    )
  }
  return items
}

/** A quick client-side duplicate check; the engine canonicalizes and decides. */
export function samePath(a: string, b: string) {
  const normalize = (path: string) =>
    path.replaceAll('\\', '/').replace(/\/+$/, '').toLowerCase()
  return normalize(a) === normalize(b)
}

export function folderName(path: string) {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path
}
