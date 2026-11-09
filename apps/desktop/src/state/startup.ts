import type { Thread } from '@/bindings/Thread'
import type { Project } from '@/bindings/Project'
import type { SidebarView } from './thread-sections'
import type { StartupView } from '@/bindings/StartupView'

export type Selection = { projectId: string | null; threadId: string | null }

/**
 * Where the app opens: the inbox, or the project and thread left open last time, or the chat
 * when that was one. Either falls back to the first project when the remembered one is gone.
 * A lone project without chats opens in place of the inbox.
 */
export function startupSelection(
  view: StartupView,
  last: Selection,
  projects: Project[],
  threads: Thread[]
): Selection & { sidebarView: SidebarView } {
  const projectId =
    (projects.find((project) => project.id === last.projectId) ?? projects[0])
      ?.id ?? null
  const lone =
    projects.length === 1 &&
    !threads.some((thread) => thread.projectId === null)
  if (view === 'inbox')
    return {
      sidebarView: lone ? 'project' : 'inbox',
      projectId,
      threadId: null
    }
  const chat = threads.find(
    (thread) => thread.id === last.threadId && thread.projectId === null
  )
  if (chat) return { sidebarView: 'chats', projectId, threadId: chat.id }
  const thread = threads.find(
    (thread) => thread.id === last.threadId && thread.projectId === projectId
  )
  return { sidebarView: 'project', projectId, threadId: thread?.id ?? null }
}
