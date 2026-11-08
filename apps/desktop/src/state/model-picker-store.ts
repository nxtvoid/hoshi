import { create } from 'zustand'

/** Shared so `/model` in the composer can open the picker. */
type ModelPickerState = {
  open: boolean
  setOpen: (open: boolean) => void
}

export const useModelPickerStore = create<ModelPickerState>()((set) => ({
  open: false,
  setOpen: (open) => set({ open })
}))
