import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { useAppStore } from '@/state/app-store'
import { PullRequestIcon } from './PullRequestIcon'
import { usePanelStore } from '@/state/panel-store'
import { PullRequestHoverCard } from './PullRequestHoverCard'
import type { ChecksState } from '@/bindings/ChecksState'
import { pullRequestProblem } from '@/lib/pull-request-problem'

const CHECKS_COLOR: Record<ChecksState, string> = {
  passing: 'text-pr-open',
  failing: 'text-status-error',
  pending: 'text-status-waiting'
}

/** The thread's pull request number and checks; opens it in the side panel. */
export function PullRequestPill({
  threadId,
  className
}: {
  threadId: string
  className?: string
}) {
  const pullRequest = useAppStore((state) => state.pullRequests[threadId])
  const openSurface = usePanelStore((state) => state.openSurface)
  if (!pullRequest) return null

  return (
    <PullRequestHoverCard
      threadId={threadId}
      pullRequest={pullRequest}
      trigger={
        <button
          type='button'
          aria-label={m.open_pull_request({ number: pullRequest.number })}
          onClick={() => openSurface(threadId, 'pullRequest')}
          className={cn(
            'border-border hover:bg-accent focus-visible:ring-ring flex h-6 shrink-0 cursor-pointer items-center gap-1 rounded-full border px-2 text-xs outline-none focus-visible:ring-2',
            className
          )}
        >
          <PullRequestIcon
            state={pullRequest.state}
            problem={pullRequestProblem(pullRequest)}
            className='size-3.5'
          />
          <span className='tabular-nums'>#{pullRequest.number}</span>
          {pullRequest.checks ? (
            <span
              aria-hidden='true'
              className={cn(
                'size-1.5 rounded-full bg-current',
                CHECKS_COLOR[pullRequest.checks]
              )}
            />
          ) : null}
        </button>
      }
    />
  )
}
