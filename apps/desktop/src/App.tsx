import { Outlet } from '@tanstack/react-router'
import { useAppStore } from '@/state/app-store'
import { Sidebar } from '@/components/sidebar/Sidebar'
import { ThreadView } from '@/components/thread/ThreadView'
import { TooltipProvider } from '@hoshi/ui/components/tooltip'
import { DialogHost } from '@/components/dialogs/DialogHost'
import { FirstRunGate } from '@/components/onboarding/FirstRunGate'
import { CommandPalette } from '@/components/palette/CommandPalette'
import { useUnseenTracking } from '@/state/unseen-store'
import { useThreadNotifications } from '@/state/thread-notifications'
import { useThreadShortcuts } from '@/components/sidebar/use-thread-shortcuts'

export function App() {
  const sidebarOpen = useAppStore((state) => state.sidebarOpen)
  useThreadShortcuts()
  useUnseenTracking()
  useThreadNotifications()

  return (
    <TooltipProvider delay={25}>
      <div className='bg-background text-foreground flex h-screen'>
        {sidebarOpen ? <Sidebar /> : null}
        <ThreadView />
        <Outlet />
      </div>
      <CommandPalette />
      <DialogHost />
      <FirstRunGate />
    </TooltipProvider>
  )
}
