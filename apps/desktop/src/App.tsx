import { Outlet } from '@tanstack/react-router'
import { useAppStore } from '@/state/app-store'
import { Sidebar } from '@/components/sidebar/Sidebar'
import { ThreadView } from '@/components/thread/ThreadView'
import { TooltipProvider } from '@hoshi/ui/components/tooltip'

export function App() {
  const sidebarOpen = useAppStore((state) => state.sidebarOpen)

  return (
    <TooltipProvider delay={400}>
      <div className='bg-background text-foreground flex h-screen'>
        {sidebarOpen ? <Sidebar /> : null}
        <ThreadView />
        <Outlet />
      </div>
    </TooltipProvider>
  )
}
