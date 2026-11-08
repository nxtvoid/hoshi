import { create } from 'zustand'

/** Everything, the repository's open pull requests to start a thread from, the project for a new thread, or the ways to add a project. */
export type PaletteMode = 'all' | 'pullRequests' | 'newThread' | 'addProject'

type PaletteState = {
  open: boolean
  mode: PaletteMode
  setOpen: (open: boolean, mode?: PaletteMode) => void
  toggle: () => void
}

export const usePaletteStore = create<PaletteState>()((set) => ({
  open: false,
  mode: 'all',
  setOpen: (open, mode = 'all') => set(open ? { open, mode } : { open }),
  toggle: () => set((state) => ({ open: !state.open, mode: 'all' }))
}))
