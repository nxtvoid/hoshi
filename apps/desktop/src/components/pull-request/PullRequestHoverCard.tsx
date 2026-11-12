import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { type ReactElement, useRef } from 'react'
import { getLocale } from '@/paraglide/runtime'
import { formatRelativeTime } from '@hoshi/utils'
import { HugeiconsIcon } from '@hugeicons/react'
import type { PullRequest } from '@/bindings/PullRequest'
import type { ChecksState } from '@/bindings/ChecksState'
import { isOthersPullRequest } from '@/state/thread-sections'
import { pullRequestProblem } from '@/lib/pull-request-problem'
import { type PullRequestTurn, pullRequestTurn } from '@/lib/pull-request-turn'
import {
  PULL_REQUEST_PROBLEM,
  PULL_REQUEST_STATE,
  PullRequestIcon
} from './PullRequestIcon'
import { usePullRequestStore } from '@/state/pull-request-store'
import { useForgeAccountsStore } from '@/state/forge-accounts-store'
import { useLinkedPullRequestStore } from '@/state/linked-pull-request-store'
import {
  CancelCircleIcon,
  Comment01Icon,
  TaskDone01Icon
} from '@hugeicons/core-free-icons'
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
  const login = useForgeAccountsStore(
    (state) =>
      state.accounts?.find((account) => account.forge === pullRequest.forge)
        ?.login ?? undefined
  )
  const own = !isOthersPullRequest(pullRequest, { [pullRequest.forge]: login })
  const turn = own ? pullRequestTurn(pullRequest) : null
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
        {!own && pullRequest.unresolvedComments > 0 ? (
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
      {turn ? <TurnSummary turn={turn} /> : null}
    </div>
  )
}

const relative = (iso: string) =>
  formatRelativeTime(iso, new Date(), getLocale())

/** Whose move the pull request waits on, and why. */
function TurnSummary({ turn }: { turn: PullRequestTurn }) {
  const yours = turn.kind === 'author'
  return (
    <div className='border-border mt-0.5 flex flex-col gap-1 border-t pt-2 text-xs'>
      <span className='flex items-center gap-1.5 font-medium'>
        <span
          className={cn(
            'size-1.5 shrink-0 rounded-full',
            yours ? 'bg-warning' : 'bg-muted-foreground'
          )}
        />
        {yours
          ? m.pr_turn_yours()
          : m.pr_turn_waiting_on({ names: turn.waitingOn.join(', ') })}
      </span>
      {turn.kind === 'reviewers' ? (
        <TurnReason icon={Comment01Icon}>
          {m.pr_turn_replied({ time: relative(turn.repliedAt) })}
        </TurnReason>
      ) : (
        <>
          {turn.ask?.ask === 'changesRequested' ? (
            <TurnReason icon={CancelCircleIcon} iconClassName='text-warning'>
              {m.pr_turn_changes_requested({
                name: turn.ask.by,
                time: relative(turn.ask.at)
              })}
            </TurnReason>
          ) : turn.ask ? (
            <TurnReason icon={Comment01Icon}>
              {m.pr_turn_commented({
                name: turn.ask.by,
                time: relative(turn.ask.at)
              })}
            </TurnReason>
          ) : null}
          {turn.threads > 0 ? (
            <TurnReason icon={Comment01Icon}>
              {m.pr_turn_threads({ count: turn.threads })}
            </TurnReason>
          ) : null}
        </>
      )}
      {turn.tasks > 0 ? (
        <TurnReason icon={TaskDone01Icon}>
          {m.pr_turn_tasks({ count: turn.tasks })}
        </TurnReason>
      ) : null}
    </div>
  )
}

function TurnReason({
  icon,
  iconClassName,
  children
}: {
  icon: typeof Comment01Icon
  iconClassName?: string
  children: string
}) {
  return (
    <span className='text-muted-foreground flex min-w-0 items-center gap-1.5 pl-3'>
      <HugeiconsIcon
        icon={icon}
        strokeWidth={2}
        className={cn('size-3 shrink-0', iconClassName)}
      />
      <span className='truncate'>{children}</span>
    </span>
  )
}
