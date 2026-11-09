import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { invoke } from '@tauri-apps/api/core'
import type { GitStatus } from '@/bindings/GitStatus'
import type { GitActionPhase } from '@/bindings/GitActionPhase'
import type { GitActionResult } from '@/bindings/GitActionResult'
import type { GitActionRequest } from '@/bindings/GitActionRequest'
import type { GitActionProgress } from '@/bindings/GitActionProgress'

/** How long a finished action's result stays in place of the button. */
const DONE_MS = 10_000

export type RunningGitAction = {
  /** `null` until the engine reports its first step. */
  phase: GitActionPhase | null
  startedAt: number
}

type GitActionState = {
  /** Each thread's checkout, read when its details show and after each action. */
  statuses: Record<string, GitStatus>
  running: Record<string, RunningGitAction>
  done: Record<string, GitActionResult>
  /** Pull requests open as drafts. Kept across restarts. */
  draft: boolean
  setDraft: (draft: boolean) => void
  loadStatus: (threadId: string) => Promise<void>
  /** Rejects with the engine's `CommandError`. */
  run: (threadId: string, request: GitActionRequest) => Promise<GitActionResult>
  receive: (progress: GitActionProgress) => void
}

const without = <T>(record: Record<string, T>, key: string) => {
  const { [key]: _, ...rest } = record
  return rest
}

export const useGitActionStore = create<GitActionState>()(
  persist(
    (set, get) => ({
      statuses: {},
      running: {},
      done: {},
      draft: false,
      setDraft: (draft) => set({ draft }),
      loadStatus: async (threadId) => {
        try {
          const status = await invoke<GitStatus>('get_git_status', { threadId })
          set((state) => ({
            statuses: { ...state.statuses, [threadId]: status }
          }))
        } catch {
          set((state) => ({ statuses: without(state.statuses, threadId) }))
        }
      },
      run: async (threadId, request) => {
        set((state) => ({
          running: {
            ...state.running,
            [threadId]: { phase: null, startedAt: Date.now() }
          },
          done: without(state.done, threadId)
        }))
        try {
          const result = await invoke<GitActionResult>('run_git_action', {
            threadId,
            request
          })
          set((state) => ({ done: { ...state.done, [threadId]: result } }))
          setTimeout(() => {
            if (get().done[threadId] === result) {
              set((state) => ({ done: without(state.done, threadId) }))
            }
          }, DONE_MS)
          return result
        } finally {
          set((state) => ({ running: without(state.running, threadId) }))
          void get().loadStatus(threadId)
        }
      },
      receive: ({ threadId, phase }) =>
        set((state) => {
          const running = state.running[threadId]
          if (!running) return state
          return {
            running: { ...state.running, [threadId]: { ...running, phase } }
          }
        })
    }),
    {
      name: 'hoshi-git-actions',
      partialize: ({ draft }) => ({ draft })
    }
  )
)
