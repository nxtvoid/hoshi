import { create } from 'zustand'
import type { LocalServer } from '@/bindings/LocalServer'
import type { ThreadServers } from '@/bindings/ThreadServers'

type ServerState = {
  /** The servers of the threads a details card watches, by thread id. */
  byThread: Record<string, LocalServer[]>
  receive: (update: ThreadServers) => void
}

export const useServerStore = create<ServerState>()((set) => ({
  byThread: {},
  receive: ({ threadId, servers }) =>
    set((state) => {
      const { [threadId]: _, ...rest } = state.byThread
      return {
        byThread: servers.length === 0 ? rest : { ...rest, [threadId]: servers }
      }
    })
}))
