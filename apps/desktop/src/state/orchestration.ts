import type { Event } from '@/bindings/Event'
import type { Thread } from '@/bindings/Thread'
import type { Project } from '@/bindings/Project'

export type OrchestrationState = {
  projects: Project[]
  threads: Thread[]
}

export function applyEvent(
  state: OrchestrationState,
  event: Event
): OrchestrationState {
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
  }
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
