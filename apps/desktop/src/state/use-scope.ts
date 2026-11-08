import { useAppStore } from './app-store'
import { inChat, scopeProjectId } from './scope'
import { useSidebarStore } from './sidebar-store'

export function useInChat() {
  const view = useSidebarStore((state) => state.view)
  return useAppStore((state) => inChat(state, view))
}

export function useScopeProjectId() {
  const view = useSidebarStore((state) => state.view)
  return useAppStore((state) => scopeProjectId(state, view))
}
