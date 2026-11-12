import { create } from 'zustand'
import type { ProjectBranch } from '@/bindings/ProjectBranch'

type ProjectBranchState = {
  /** The branch each project's root folder is on, by project id. */
  byProject: Record<string, string | null>
  /** Branch mismatches the user dismissed in the chat, kept for this session. */
  dismissed: Record<string, true>
  seed: (branches: ProjectBranch[]) => void
  receive: (update: ProjectBranch) => void
  dismiss: (key: string) => void
}

export const useProjectBranchStore = create<ProjectBranchState>()((set) => ({
  byProject: {},
  dismissed: {},
  seed: (branches) =>
    set({
      byProject: Object.fromEntries(
        branches.map(({ projectId, branch }) => [projectId, branch])
      )
    }),
  receive: ({ projectId, branch }) =>
    set((state) => ({
      byProject: { ...state.byProject, [projectId]: branch }
    })),
  dismiss: (key) =>
    set((state) => ({ dismissed: { ...state.dismissed, [key]: true } }))
}))
