import { cn } from '@hoshi/utils'
import { lazy, Suspense } from 'react'
import { PanelShown } from './panel-shown'
import { usePanelStore } from '@/state/panel-store'

const RightPanelView = lazy(() =>
  import('./RightPanelView').then((module) => ({
    default: module.RightPanelView
  }))
)

export function RightPanel({
  threadId,
  shown
}: {
  threadId: string
  /** Kept mounted while closed, hidden, so it opens again as it was. */
  shown: boolean
}) {
  return (
    <div className={shown ? 'contents' : 'hidden'} inert={!shown}>
      <PanelShown.Provider value={shown}>
        <Suspense fallback={<PanelPlaceholder />}>
          <RightPanelView threadId={threadId} />
        </Suspense>
      </PanelShown.Provider>
    </div>
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
