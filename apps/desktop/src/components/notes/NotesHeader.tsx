import type { ReactNode } from 'react'
import { m } from '@/paraglide/messages'
import { useAppStore } from '@/state/app-store'
import { NotesProjectPicker } from './NotesProjectPicker'
import { HugeiconsIcon } from '@hugeicons/react'
import { SidebarLeftIcon } from '@hugeicons/core-free-icons'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { WindowControls } from '@/components/window/WindowControls'

/**
 * The bar over the notes' main view: the project whose notes are listed, then what it
 * shows, on the left; its actions on the right.
 */
export function NotesHeader({
  children,
  actions
}: {
  children: ReactNode
  actions?: ReactNode
}) {
  const sidebarOpen = useAppStore((state) => state.sidebarOpen)
  const toggleSidebar = useAppStore((state) => state.toggleSidebar)

  return (
    <header
      data-tauri-drag-region
      className='border-border flex h-10 shrink-0 items-center gap-2 border-b pl-3'
    >
      {sidebarOpen ? null : (
        <IconButton label={m.toggle_sidebar()} onClick={toggleSidebar}>
          <HugeiconsIcon icon={SidebarLeftIcon} strokeWidth={2} />
        </IconButton>
      )}
      <NotesProjectPicker />
      <span aria-hidden className='text-muted-foreground/60 text-sm'>
        /
      </span>
      {children}
      <div className='ml-auto flex h-full shrink-0 items-center gap-1'>
        {actions}
        <WindowControls />
      </div>
    </header>
  )
}
