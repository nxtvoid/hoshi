import type { ReactElement, ReactNode } from 'react'
import { ThreadSummary } from './ThreadSummary'
import { useAppStore } from '@/state/app-store'
import {
  createHoverCardHandle,
  HoverCard,
  HoverCardContent,
  HoverCardTrigger
} from '@hoshi/ui/components/hover-card'

/** Resting this long on a row shows its thread's summary. */
const OPEN_DELAY_MS = 400

/**
 * One card for every thread row: once open, moving to another row moves the card there and
 * shows that thread at once.
 */
const threadCard = createHoverCardHandle<string>()

/** A pull request pill has its own card; the thread's stays closed while the pointer is on one. */
let overPullRequest = false

export const pullRequestPillHover = {
  onPointerEnter: () => {
    overPullRequest = true
    threadCard.close()
  },
  onPointerLeave: () => {
    overPullRequest = false
  }
}

/** The card itself, rendered once next to the rows that open it. */
export function ThreadCardHost() {
  return (
    <HoverCard
      handle={threadCard}
      onOpenChange={(open, details) => {
        if (open && overPullRequest) details.cancel()
      }}
    >
      {({ payload }) => (
        <HoverCardContent
          side='right'
          align='start'
          sideOffset={12}
          className='w-64 p-3'
          positionerClassName='transition-[top,left,transform] duration-150 ease-out data-instant:transition-none'
        >
          {payload ? <ThreadCardBody threadId={payload} /> : null}
        </HoverCardContent>
      )}
    </HoverCard>
  )
}

function ThreadCardBody({ threadId }: { threadId: string }) {
  const thread = useAppStore((state) =>
    state.threads.find((candidate) => candidate.id === threadId)
  )
  return thread ? <ThreadSummary thread={thread} /> : null
}

/** Makes `render` open the shared card for `threadId`. */
export function ThreadCardTrigger({
  threadId,
  render,
  children
}: {
  threadId: string
  render: ReactElement
  children: ReactNode
}) {
  return (
    <HoverCardTrigger
      handle={threadCard}
      payload={threadId}
      delay={OPEN_DELAY_MS}
      closeDelay={100}
      render={render}
    >
      {children}
    </HoverCardTrigger>
  )
}
