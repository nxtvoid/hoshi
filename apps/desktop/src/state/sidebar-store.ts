import { create } from 'zustand'
import { persist } from 'zustand/middleware'
import type {
  SectionId,
  SidebarView,
  ThreadFilter,
  ThreadSort
} from './thread-sections'

export type { SidebarView, ThreadFilter, ThreadSort }

export const SIDEBAR_WIDTH = { min: 220, max: 420, default: 272 } as const

type SidebarState = {
  threadSort: ThreadSort
  view: SidebarView
  /**
   * The sidebar lists the notes of where `view` works and the main view shows one, in
   * place of its threads, which come back on leaving.
   */
  notesOpen: boolean
  filter: ThreadFilter
  width: number
  /** Sections the user folded; inactive and done start folded. */
  collapsed: Partial<Record<SectionId, boolean>>
  setThreadSort: (threadSort: ThreadSort) => void
  /** Shows the threads of `view`, leaving the notes. */
  setView: (view: SidebarView) => void
  setNotesOpen: (notesOpen: boolean) => void
  setFilter: (filter: ThreadFilter) => void
  setWidth: (width: number) => void
  toggleSection: (section: SectionId) => void
}

export const useSidebarStore = create<SidebarState>()(
  persist(
    (set) => ({
      threadSort: 'activity',
      view: 'project',
      notesOpen: false,
      filter: 'all',
      width: SIDEBAR_WIDTH.default,
      collapsed: { inactive: true, done: true },
      setThreadSort: (threadSort) => set({ threadSort }),
      setView: (view) => set({ view, notesOpen: false }),
      setNotesOpen: (notesOpen) => set({ notesOpen }),
      setFilter: (filter) => set({ filter }),
      setWidth: (width) =>
        set({
          width: Math.round(
            Math.min(SIDEBAR_WIDTH.max, Math.max(SIDEBAR_WIDTH.min, width))
          )
        }),
      toggleSection: (section) =>
        set((state) => ({
          collapsed: {
            ...state.collapsed,
            [section]: !state.collapsed[section]
          }
        }))
    }),
    {
      name: 'hoshi-sidebar',
      partialize: ({ notesOpen: _, ...kept }) => kept
    }
  )
)
