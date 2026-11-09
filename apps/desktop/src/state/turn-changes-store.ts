import { create } from 'zustand'

export const OPEN_CHANGES_LIMIT = 25

type TurnChangesState = {
  /** Cards the user opened or closed this session, by turn id. */
  open: Record<string, boolean>
  setOpen: (turnId: string, open: boolean) => void
}

export const useTurnChangesStore = create<TurnChangesState>()((set) => ({
  open: {},
  setOpen: (turnId, open) =>
    set((state) => ({ open: { ...state.open, [turnId]: open } }))
}))

export function isChangesCardOpen(
  chosen: boolean | undefined,
  fileCount: number
) {
  return chosen ?? fileCount <= OPEN_CHANGES_LIMIT
}
