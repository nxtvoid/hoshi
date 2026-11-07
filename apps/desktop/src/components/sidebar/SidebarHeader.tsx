import { ThemeMenu } from './ThemeMenu'
import { m } from '@/paraglide/messages'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { IconButton } from '@/components/IconButton'
import {
  FilterHorizontalIcon,
  FolderAddIcon,
  PencilEdit02Icon,
  Search01Icon,
  SidebarLeftIcon
} from '@hugeicons/core-free-icons'

export function SidebarHeader() {
  const search = useAppStore((state) => state.search)
  const setSearch = useAppStore((state) => state.setSearch)
  const toggleSidebar = useAppStore((state) => state.toggleSidebar)
  const createThread = useAppStore((state) => state.createThread)
  const hasProject = useAppStore((state) => state.selectedProjectId !== null)

  return (
    <header className='border-sidebar-border bg-starfield flex flex-col border-b px-2 pb-2.5'>
      <div data-tauri-drag-region className='flex h-10 items-center gap-1'>
        <IconButton label={m.toggle_sidebar()} onClick={toggleSidebar}>
          <HugeiconsIcon icon={SidebarLeftIcon} strokeWidth={2} />
        </IconButton>
        <span
          data-tauri-drag-region
          className='text-sm font-semibold tracking-tight'
        >
          hoshi
        </span>
        <div className='ml-auto'>
          <ThemeMenu />
        </div>
      </div>
      <div className='flex items-center gap-1'>
        <label className='text-sidebar-foreground/70 hover:bg-sidebar-accent/60 focus-within:bg-sidebar-accent flex h-8 min-w-0 flex-1 items-center gap-2 rounded-lg px-2 text-sm'>
          <HugeiconsIcon
            icon={Search01Icon}
            strokeWidth={2}
            className='size-4 shrink-0'
          />
          <input
            type='search'
            value={search}
            onChange={(event) => setSearch(event.currentTarget.value)}
            placeholder={m.search_threads()}
            aria-label={m.search_threads()}
            className='placeholder:text-sidebar-foreground/50 min-w-0 flex-1 bg-transparent outline-none'
          />
        </label>
        <IconButton label={m.filter_threads()} disabled>
          <HugeiconsIcon icon={FilterHorizontalIcon} strokeWidth={2} />
        </IconButton>
        <IconButton label={m.add_project()} disabled>
          <HugeiconsIcon icon={FolderAddIcon} strokeWidth={2} />
        </IconButton>
        <IconButton
          label={m.new_thread()}
          onClick={createThread}
          disabled={!hasProject}
        >
          <HugeiconsIcon icon={PencilEdit02Icon} strokeWidth={2} />
        </IconButton>
      </div>
    </header>
  )
}
