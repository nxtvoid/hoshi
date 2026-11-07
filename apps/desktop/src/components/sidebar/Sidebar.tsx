import { ThreadList } from './ThreadList'
import { ProjectRail } from './ProjectRail'
import { SidebarFooter } from './SidebarFooter'
import { SidebarHeader } from './SidebarHeader'

export function Sidebar() {
  return (
    <aside className='border-sidebar-border bg-sidebar text-sidebar-foreground flex w-80 shrink-0 flex-col border-r'>
      <SidebarHeader />
      <div className='flex min-h-0 flex-1'>
        <ProjectRail />
        <ThreadList />
      </div>
      <SidebarFooter />
    </aside>
  )
}
