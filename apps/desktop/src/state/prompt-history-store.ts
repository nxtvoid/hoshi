import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import { remember } from '@/lib/prompt-history'

const LIMIT = 200

/** Prompts sent from any thread, oldest first, recalled with ↑ in the composer. */
type PromptHistoryState = {
  entries: string[]
  add: (prompt: string) => void
}

export const usePromptHistoryStore = create<PromptHistoryState>()(
  persist(
    (set) => ({
      entries: [],
      add: (prompt) =>
        set((state) => ({ entries: remember(state.entries, prompt, LIMIT) }))
    }),
    { name: 'hoshi-prompt-history' }
  )
)
