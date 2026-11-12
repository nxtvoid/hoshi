import { m } from '@/paraglide/messages'
import { Outlet } from '@tanstack/react-router'
import { useAppStore } from '@/state/app-store'
import { Sidebar } from '@/components/sidebar/Sidebar'
import { ThreadView } from '@/components/thread/ThreadView'
import { TooltipProvider } from '@hoshi/ui/components/tooltip'
import { DialogHost } from '@/components/dialogs/DialogHost'
import { FirstRunGate } from '@/components/onboarding/FirstRunGate'
import { CommandPalette } from '@/components/palette/CommandPalette'
import { EditContextMenu } from '@/components/window/EditContextMenu'
import { useUnseenTracking } from '@/state/unseen-store'
import { useThreadNotifications } from '@/state/thread-notifications'
import { useThreadShortcuts } from '@/components/sidebar/use-thread-shortcuts'
import {
  AnchoredToastProvider,
  type ToastLabels,
  ToastProvider
} from '@hoshi/ui/components/toast'

const toastLabels = (): ToastLabels => ({
  dismiss: m.toast_dismiss(),
  copyError: m.toast_copy_error(),
  copiedError: m.toast_copied_error(),
  showDetails: m.toast_show_details(),
  hideDetails: m.toast_hide_details()
})

export function App() {
  const sidebarOpen = useAppStore((state) => state.sidebarOpen)
  const threadId = useAppStore((state) => state.selectedThreadId)
  const labels = toastLabels()
  useThreadShortcuts()
  useUnseenTracking()
  useThreadNotifications()

  return (
    <TooltipProvider>
      <ToastProvider
        labels={labels}
        activeScope={threadId}
        viewportClassName='[--toast-header-offset:--spacing(10)]'
      >
        <AnchoredToastProvider labels={labels} activeScope={threadId}>
          <div className='bg-background text-foreground flex h-screen'>
            <Sidebar shown={sidebarOpen} />
            <ThreadView />
            <Outlet />
          </div>
          <CommandPalette />
          <DialogHost />
          <FirstRunGate />
          <EditContextMenu />
        </AnchoredToastProvider>
      </ToastProvider>
    </TooltipProvider>
  )
}
