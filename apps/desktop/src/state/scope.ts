import type { Thread } from '@/bindings/Thread'
import type { SidebarView } from './thread-sections'

type ScopeSource = {
  threads: Thread[]
  selectedThreadId: string | null
  selectedProjectId: string | null
}

/**
 * Whether the main view works in a chat: the open thread is one, or, with no thread open,
 * the sidebar lists the chats.
 */
export function inChat(state: ScopeSource, view: SidebarView): boolean {
  const thread = state.threads.find(
    (candidate) => candidate.id === state.selectedThreadId
  )
  return thread ? thread.projectId === null : view === 'chats'
}

/** The project the main view works in; `null` in a chat or with none selected. */
export function scopeProjectId(state: ScopeSource, view: SidebarView) {
  return inChat(state, view) ? null : state.selectedProjectId
}

/** The sidebar view that lists `thread`. */
export function viewOf(thread: Thread): SidebarView {
  return thread.projectId === null ? 'chats' : 'project'
}
