import { Composer } from './Composer'
import { m } from '@/paraglide/messages'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { LogoMark } from '@/components/brand/Logo'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { WindowControls } from '@/components/window/WindowControls'
import { GitBranchIcon, SidebarLeftIcon } from '@hugeicons/core-free-icons'

export function ThreadView() {
  const sidebarOpen = useAppStore((state) => state.sidebarOpen)
  const toggleSidebar = useAppStore((state) => state.toggleSidebar)
  const hasProject = useAppStore((state) => state.selectedProjectId !== null)
  const thread = useAppStore((state) =>
    state.threads.find((candidate) => candidate.id === state.selectedThreadId)
  )

  return (
    <main className='flex min-w-0 flex-1 flex-col'>
      <header
        data-tauri-drag-region
        className='border-border flex h-10 shrink-0 items-center gap-2 border-b pl-3'
      >
        {sidebarOpen ? null : (
          <IconButton label={m.toggle_sidebar()} onClick={toggleSidebar}>
            <HugeiconsIcon icon={SidebarLeftIcon} strokeWidth={2} />
          </IconButton>
        )}
        {thread ? (
          <>
            <h1 data-tauri-drag-region className='truncate text-sm font-medium'>
              {thread.title || m.untitled_thread()}
            </h1>
            {thread.branch ? (
              <span
                data-tauri-drag-region
                className='text-muted-foreground flex shrink-0 items-center gap-1 text-xs'
              >
                <HugeiconsIcon
                  icon={GitBranchIcon}
                  strokeWidth={2}
                  className='size-3'
                />
                {thread.branch}
              </span>
            ) : null}
          </>
        ) : null}
        <div className='ml-auto h-full'>
          <WindowControls />
        </div>
      </header>
      <div className='text-muted-foreground flex flex-1 flex-col items-center justify-center gap-4 overflow-y-auto text-sm'>
        <LogoMark className='text-muted-foreground/25 size-12' />
        <p>
          {thread
            ? m.no_messages()
            : hasProject
              ? m.new_thread_prompt()
              : m.no_project_selected()}
        </p>
      </div>
      {hasProject ? (
        <div className='p-3'>
          <Composer />
        </div>
      ) : null}
    </main>
  )
}
