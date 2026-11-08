import { create } from 'zustand'

type StoppingState = {
  /** When the user asked each thread's turn to stop, until the turn ends. */
  since: Record<string, number>
  start: (threadId: string) => void
  end: (threadId: string) => void
}

export const useStoppingStore = create<StoppingState>()((set) => ({
  since: {},
  start: (threadId) =>
    set((state) => ({ since: { ...state.since, [threadId]: Date.now() } })),
  end: (threadId) =>
    set((state) => {
      if (!(threadId in state.since)) return state
      const { [threadId]: _, ...rest } = state.since
      return { since: rest }
    })
}))
