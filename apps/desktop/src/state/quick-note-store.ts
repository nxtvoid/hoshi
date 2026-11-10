import { create } from 'zustand'

type QuickNoteState = {
  /** The dialog that takes a note from anywhere in the app. */
  open: boolean
  setOpen: (open: boolean) => void
}

export const useQuickNoteStore = create<QuickNoteState>()((set) => ({
  open: false,
  setOpen: (open) => set({ open })
}))
