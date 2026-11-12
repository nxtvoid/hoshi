import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { type ReactElement, useRef } from 'react'
import { getLocale } from '@/paraglide/runtime'
import { formatRelativeTime } from '@hoshi/utils'
import { HugeiconsIcon } from '@hugeicons/react'
import type { PullRequest } from '@/bindings/PullRequest'
import type { ChecksState } from '@/bindings/ChecksState'
import { pullRequestProblem } from '@/lib/pull-request-problem'
import {
  PULL_REQUEST_PROBLEM,
  PULL_REQUEST_STATE,
  PullRequestIcon
} from './PullRequestIcon'
import { Comment01Icon } from '@hugeicons/core-free-icons'
import { usePullRequestStore } from '@/state/pull-request-store'
import { useLinkedPullRequestStore } from '@/state/linked-pull-request-store'
import {
  HoverCard,
  HoverCardContent,
  HoverCardTrigger
} from '@hoshi/ui/components/hover-card'

const HOVER_DELAY_MS = 400
/** Resting this long on the trigger starts reading the detail, before the card opens. */
const PREFETCH_DELAY_MS = 150

const CHECKS: Record<ChecksState, { label: () => string; color: string }> = {
  passing: { label: m.checks_passing, color: 'text-pr-open' },
  failing: { label: m.checks_failing, color: 'text-status-error' },
  pending: { label: m.checks_pending, color: 'text-status-waiting' }
}

/** `owner/name` from a pull request's web address, or nothing if it has another shape. */
function repositoryOf(url: string) {
  try {
    const [owner, name] = new URL(url).pathname.split('/').filter(Boolean)
    return owner && name ? `${owner}/${name}` : null
  } catch {
    return null
  }
}

/** Shows a pull request's summary when the pointer rests on `trigger`. */
export function PullRequestHoverCard({
  threadId,
  pullRequest,
  trigger,
  side = 'bottom'
}: {
  threadId: string
  pullRequest: PullRequest
  /** The element hovering opens the card from; it keeps its own click behavior. */
  trigger: ReactElement
  side?: 'top' | 'bottom' | 'left' | 'right'
}) {
  const prefetch = usePullRequestStore((store) => store.prefetch)
  const prefetchTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const cancelPrefetch = () => {
    if (prefetchTimer.current) clearTimeout(prefetchTimer.current)
    prefetchTimer.current = null
  }

  return (
    <HoverCard>
      <HoverCardTrigger
        delay={HOVER_DELAY_MS}
        closeDelay={100}
        render={trigger}
        onPointerEnter={() => {
          cancelPrefetch()
          prefetchTimer.current = setTimeout(
            () => prefetch(threadId),
            PREFETCH_DELAY_MS
          )
        }}
        onPointerLeave={cancelPrefetch}
      />
      <HoverCardContent
        side={side}
        align='start'
        className='w-80 p-3'
        sideOffset={15}
      >
        <PullRequestSummary pullRequest={pullRequest} />
      </HoverCardContent>
    </HoverCard>
  )
}

/** Shows the pull request a web link points to when the pointer rests on `trigger`. */
export function PullRequestLinkCard({
  url,
  trigger
}: {
  url: string
  trigger: ReactElement
}) {
  const pullRequest = useLinkedPullRequestStore((store) => store.byUrl[url])
  const load = useLinkedPullRequestStore((store) => store.load)

  return (
    <HoverCard>
      <HoverCardTrigger
        delay={HOVER_DELAY_MS}
        closeDelay={100}
        render={trigger}
        onPointerEnter={() => load(url)}
        onFocus={() => load(url)}
      />
      <HoverCardContent
        side='top'
        align='start'
        className={cn(
          pullRequest ? 'w-80 p-3' : 'w-auto max-w-sm px-2.5 py-1.5'
        )}
      >
        {pullRequest ? (
          <PullRequestSummary pullRequest={pullRequest} />
        ) : (
          <span className='text-muted-foreground text-xs break-all'>{url}</span>
        )}
      </HoverCardContent>
    </HoverCard>
  )
}

function PullRequestSummary({ pullRequest }: { pullRequest: PullRequest }) {
  const repository = repositoryOf(pullRequest.url)
  const problem = pullRequestProblem(pullRequest)
  const label = problem
    ? PULL_REQUEST_PROBLEM[problem].label
    : PULL_REQUEST_STATE[pullRequest.state].label
  const checks = pullRequest.checks ? CHECKS[pullRequest.checks] : null
  return (
    <div className='flex flex-col gap-1.5'>
      <div className='text-muted-foreground flex min-w-0 items-center gap-1.5 text-xs'>
        {repository ? <span className='truncate'>{repository}</span> : null}
        <span className='shrink-0 tabular-nums'>#{pullRequest.number}</span>
        <span aria-hidden='true'>·</span>
        <span className='flex shrink-0 items-center gap-1'>
          <PullRequestIcon
            state={pullRequest.state}
            problem={problem}
            className='size-3'
          />
          {label()}
        </span>
      </div>
      <p className='text-sm leading-snug font-medium text-pretty'>
        {pullRequest.title}
      </p>
      <p className='text-muted-foreground truncate text-xs'>
        {pullRequest.author ? (
          <span className='text-foreground'>{pullRequest.author}</span>
        ) : null}
        {pullRequest.author ? ' · ' : null}
        {m.pr_updated({
          time: formatRelativeTime(
            pullRequest.updatedAt,
            new Date(),
            getLocale()
          )
        })}
      </p>
      <div className='text-muted-foreground flex flex-wrap items-center gap-x-3 gap-y-1 text-xs'>
        <span className='min-w-0 truncate font-mono'>
          {pullRequest.baseBranch} ← {pullRequest.headBranch}
        </span>
        <span className='tabular-nums'>
          <span className='text-pr-open'>+{pullRequest.additions}</span>{' '}
          <span className='text-status-error'>−{pullRequest.deletions}</span>
        </span>
        {checks ? (
          <span className={cn('flex items-center gap-1', checks.color)}>
            <span className='size-1.5 rounded-full bg-current' />
            {checks.label()}
          </span>
        ) : null}
        {pullRequest.unresolvedComments > 0 ? (
          <span className='flex items-center gap-1'>
            <HugeiconsIcon
              icon={Comment01Icon}
              strokeWidth={2}
              className='size-3'
            />
            {m.unresolved_comments({
              count: pullRequest.unresolvedComments
            })}
          </span>
        ) : null}
      </div>
    </div>
  )
}
