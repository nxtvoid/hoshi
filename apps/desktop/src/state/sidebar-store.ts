import { create } from 'zustand'
import { persist } from 'zustand/middleware'

export type ThreadSort = 'activity' | 'created'

type SidebarState = {
  threadSort: ThreadSort
  setThreadSort: (threadSort: ThreadSort) => void
}

export const useSidebarStore = create<SidebarState>()(
  persist(
    (set) => ({
      threadSort: 'activity',
      setThreadSort: (threadSort) => set({ threadSort })
    }),
    { name: 'hoshi-sidebar' }
  )
)
