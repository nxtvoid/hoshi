import { useEffect } from 'react'
import { createFileRoute } from '@tanstack/react-router'
import { useAppStore } from '@/state/app-store'
import { useUnseenStore } from '@/state/unseen-store'

// The chat view always renders in the layout; the URL only adds overlays.
export const Route = createFileRoute('/')({ component: Uncovered })

/** Back from an overlay, the open thread shows again, so its finished turn is seen. */
function Uncovered() {
  useEffect(() => {
    const { selectedThreadId } = useAppStore.getState()
    if (selectedThreadId && document.hasFocus()) {
      useUnseenStore.getState().see(selectedThreadId)
    }
  }, [])
  return null
}
