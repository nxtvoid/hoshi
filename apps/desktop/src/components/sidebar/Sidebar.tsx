import { ThreadList } from './ThreadList'
import { ProjectRail } from './ProjectRail'
import { m } from '@/paraglide/messages'
import { SidebarFooter } from './SidebarFooter'
import { SidebarHeader } from './SidebarHeader'
import { NotesSidebar } from '@/components/notes/NotesSidebar'
import { ResizeHandle } from '@hoshi/ui/components/resize-handle'
import { SIDEBAR_WIDTH, useSidebarStore } from '@/state/sidebar-store'

export function Sidebar() {
  const width = useSidebarStore((state) => state.width)
  const setWidth = useSidebarStore((state) => state.setWidth)
  const notesOpen = useSidebarStore((state) => state.notesOpen)

  return (
    <aside
      style={{ width }}
      className='border-sidebar-border bg-sidebar text-sidebar-foreground relative flex shrink-0 flex-col border-r'
    >
      <SidebarHeader />
      <div className='flex min-h-0 flex-1'>
        <ProjectRail />
        {notesOpen ? <NotesSidebar /> : <ThreadList />}
      </div>
      <SidebarFooter />
      <ResizeHandle
        edge='right'
        label={m.resize_sidebar()}
        value={width}
        min={SIDEBAR_WIDTH.min}
        max={SIDEBAR_WIDTH.max}
        defaultValue={SIDEBAR_WIDTH.default}
        onChange={setWidth}
        className='hover:bg-sidebar-ring/40 focus-visible:bg-sidebar-ring/60 active:bg-sidebar-ring/60'
      />
    </aside>
  )
}
