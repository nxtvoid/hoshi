import { useAppStore } from '@/state/app-store'
import { Sidebar } from '@/components/sidebar/Sidebar'
import { TooltipProvider } from '@/components/ui/tooltip'
import { ThreadView } from '@/components/thread/ThreadView'

export function App() {
  const sidebarOpen = useAppStore((state) => state.sidebarOpen)

  return (
    <TooltipProvider delay={400}>
      <div className='bg-background text-foreground flex h-screen'>
        {sidebarOpen ? <Sidebar /> : null}
        <ThreadView />
      </div>
    </TooltipProvider>
  )
}
