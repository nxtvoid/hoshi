import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import type { Editor } from '@/bindings/Editor'

type EditorState = {
  /** The editor "Open in" uses; the first installed one until the user picks. */
  preferred: Editor | null
  setPreferred: (editor: Editor) => void
}

export const useEditorStore = create<EditorState>()(
  persist(
    (set) => ({
      preferred: null,
      setPreferred: (preferred) => set({ preferred })
    }),
    { name: 'hoshi-editor' }
  )
)
