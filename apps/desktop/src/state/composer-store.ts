import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import type { Effort } from '@/bindings/Effort'

export type DraftWorkspace = 'local' | 'newWorktree'

/** What a new thread starts with: the last choices made in the composer. */
type ComposerState = {
  model: string | null
  effort: Effort | null
  workspace: DraftWorkspace
  setModel: (model: string | null, effort: Effort | null) => void
  setWorkspace: (workspace: DraftWorkspace) => void
}

export const useComposerStore = create<ComposerState>()(
  persist(
    (set) => ({
      model: null,
      effort: null,
      workspace: 'local',
      setModel: (model, effort) => set({ model, effort }),
      setWorkspace: (workspace) => set({ workspace })
    }),
    { name: 'hoshi-composer' }
  )
)

/** Claude Code model aliases; the CLI resolves each to its latest model. */
export const CLAUDE_MODELS = [
  { value: 'fable', label: 'Fable' },
  { value: 'opus', label: 'Opus' },
  { value: 'sonnet', label: 'Sonnet' },
  { value: 'haiku', label: 'Haiku' }
] as const

export const EFFORTS: readonly Effort[] = [
  'low',
  'medium',
  'high',
  'xhigh',
  'max'
]
