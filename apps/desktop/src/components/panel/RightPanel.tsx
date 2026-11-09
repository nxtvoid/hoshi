import { cn } from '@hoshi/utils'
import { lazy, Suspense } from 'react'
import { usePanelStore } from '@/state/panel-store'

const RightPanelView = lazy(() =>
  import('./RightPanelView').then((module) => ({
    default: module.RightPanelView
  }))
)

export function RightPanel({ threadId }: { threadId: string }) {
  return (
    <Suspense fallback={<PanelPlaceholder />}>
      <RightPanelView threadId={threadId} />
    </Suspense>
  )
}

/** Holds the panel's place while its code loads, so the chat doesn't jump. */
function PanelPlaceholder() {
  const width = usePanelStore((state) => state.width)
  const maximized = usePanelStore((state) => state.maximized)

  return (
    <aside
      style={maximized ? undefined : { width }}
      className={cn(
        'bg-background relative flex min-h-0 flex-col',
        maximized ? 'min-w-0 flex-1' : 'border-border shrink-0 border-l'
      )}
    />
  )
}
