import { create } from 'zustand'
import type { ThemeAppearance } from './themes'

export type ThemeEditorSession = {
  /** The custom theme being edited, or `null` for a new one. */
  editingThemeId: string | null
  /** The theme a new one starts from. */
  seedThemeId: string | null
  seedName: string | null
  initialAppearance: ThemeAppearance
  /** Remounts the editor for each opening. */
  id: number
}

type ThemeEditorState = {
  session: ThemeEditorSession | null
  openThemeEditor: (session: Omit<ThemeEditorSession, 'id'>) => void
  closeThemeEditor: () => void
}

let nextSessionId = 0

export const useThemeEditorStore = create<ThemeEditorState>()((set) => ({
  session: null,
  openThemeEditor: (session) =>
    set({ session: { ...session, id: ++nextSessionId } }),
  closeThemeEditor: () => set({ session: null })
}))
