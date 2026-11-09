import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'
import type { WorktreeSetupRun } from '@/bindings/WorktreeSetupRun'

type WorktreeSetupState = {
  /** The latest setup of each thread's worktree since the app started. */
  runs: Record<string, WorktreeSetupRun>
  /** Threads whose finished setup the user put away. */
  dismissed: Record<string, boolean>
  load: () => Promise<void>
  receive: (run: WorktreeSetupRun) => void
  dismiss: (threadId: string) => void
}

export const useWorktreeSetupStore = create<WorktreeSetupState>()((set) => ({
  runs: {},
  dismissed: {},
  load: async () => {
    const runs = await invoke<WorktreeSetupRun[]>('worktree_setups').catch(
      () => []
    )
    set((state) => ({
      runs: {
        ...Object.fromEntries(runs.map((run) => [run.threadId, run])),
        ...state.runs
      }
    }))
  },
  receive: (run) =>
    set((state) => ({
      runs: { ...state.runs, [run.threadId]: run },
      dismissed:
        run.status === 'running'
          ? { ...state.dismissed, [run.threadId]: false }
          : state.dismissed
    })),
  dismiss: (threadId) =>
    set((state) => ({ dismissed: { ...state.dismissed, [threadId]: true } }))
}))
