import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import type { Effort } from '@/bindings/Effort'
import type { AccessMode } from '@/bindings/AccessMode'

export type DraftWorkspace = 'local' | 'newWorktree'

export type ModelChoice = {
  model: string | null
  effort: Effort | null
  fastMode: boolean
  ultracode: boolean
}

export type AccessChoice = {
  accessMode: AccessMode
  planMode: boolean
}

/** What a new thread starts with: the last choices made in the composer. */
type ComposerState = ModelChoice &
  AccessChoice & {
    workspace: DraftWorkspace
    setModel: (choice: ModelChoice) => void
    setAccess: (choice: AccessChoice) => void
    setWorkspace: (workspace: DraftWorkspace) => void
  }

export const useComposerStore = create<ComposerState>()(
  persist(
    (set) => ({
      model: null,
      effort: null,
      fastMode: false,
      ultracode: false,
      accessMode: 'acceptEdits',
      planMode: false,
      workspace: 'local',
      setModel: (choice) => set(choice),
      setAccess: (choice) => set(choice),
      setWorkspace: (workspace) => set({ workspace })
    }),
    { name: 'hoshi-composer' }
  )
)
