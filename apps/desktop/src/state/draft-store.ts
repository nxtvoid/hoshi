import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import type { JSONContent } from '@tiptap/core'

/** Where a draft belongs: a thread, or a thread not started yet in a project or among the chats. */
export function draftKey(
  threadId: string | null,
  projectId: string | null,
  chat: boolean
) {
  if (threadId) return threadId
  return chat ? 'new:chat' : `new:${projectId ?? ''}`
}

type DraftState = {
  /** Unsent composer text, as the editor's document, by draft key. */
  drafts: Record<string, JSONContent>
  /** `null` forgets the draft. */
  save: (key: string, doc: JSONContent | null) => void
}

export const useDraftStore = create<DraftState>()(
  persist(
    (set) => ({
      drafts: {},
      save: (key, doc) =>
        set((state) => {
          if (doc === null) {
            if (!(key in state.drafts)) return state
            const { [key]: _, ...rest } = state.drafts
            return { drafts: rest }
          }
          return { drafts: { ...state.drafts, [key]: doc } }
        })
    }),
    { name: 'hoshi-drafts' }
  )
)
