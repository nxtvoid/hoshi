import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { PullRequestDetail } from '@/bindings/PullRequestDetail'
import type { ThreadPullRequestDetail } from '@/bindings/ThreadPullRequestDetail'

type PullRequestDetailState = {
  /** The last detail shown for each thread, by thread id. */
  details: Record<string, PullRequestDetail>
  /** Threads whose last read failed; only matters while nothing is cached. */
  failed: Record<string, true>
  /** Shows what the engine has at once; a fresh read arrives later through `receive`. */
  load: (threadId: string) => Promise<void>
  /** Warms the detail and files before they are opened. */
  prefetch: (threadId: string) => void
  receive: (update: ThreadPullRequestDetail) => void
}

export const usePullRequestStore = create<PullRequestDetailState>()(
  (set, get) => ({
    details: {},
    failed: {},
    load: async (threadId) => {
      try {
        const detail = await invoke<PullRequestDetail | null>(
          'get_pull_request_detail',
          { threadId }
        )
        if (detail) get().receive({ threadId, detail })
      } catch (error) {
        console.error('hoshi: failed to load a pull request detail', error)
      }
    },
    prefetch: (threadId) => {
      void invoke('prefetch_pull_request', { threadId }).catch((error) =>
        console.error('hoshi: failed to prefetch a pull request', error)
      )
    },
    receive: ({ threadId, detail }) =>
      set((state) => {
        if (detail) {
          const { [threadId]: _, ...failed } = state.failed
          return { details: { ...state.details, [threadId]: detail }, failed }
        }
        return { failed: { ...state.failed, [threadId]: true } }
      })
  })
)
