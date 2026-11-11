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
  filter: ThreadFilter
  width: number
  /** Sections the user folded; inactive and done start folded. */
  collapsed: Partial<Record<SectionId, boolean>>
  /** Project ids in the order the user dragged them into on the rail. */
  projectOrder: string[]
  setThreadSort: (threadSort: ThreadSort) => void
  setView: (view: SidebarView) => void
  setFilter: (filter: ThreadFilter) => void
  setWidth: (width: number) => void
  toggleSection: (section: SectionId) => void
  setProjectOrder: (projectOrder: string[]) => void
}

export const useSidebarStore = create<SidebarState>()(
  persist(
    (set) => ({
      threadSort: 'activity',
      view: 'project',
      filter: 'all',
      width: SIDEBAR_WIDTH.default,
      collapsed: { inactive: true, done: true },
      projectOrder: [],
      setThreadSort: (threadSort) => set({ threadSort }),
      setView: (view) => set({ view }),
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
        })),
      setProjectOrder: (projectOrder) => set({ projectOrder })
    }),
    { name: 'hoshi-sidebar' }
  )
)
