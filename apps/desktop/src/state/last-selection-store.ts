import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import type { Selection } from './startup'

/** The project and thread open last, for opening the app where the user left it. */
export const useLastSelectionStore = create<Selection>()(
  persist((): Selection => ({ projectId: null, threadId: null }), {
    name: 'hoshi-last-selection'
  })
)
