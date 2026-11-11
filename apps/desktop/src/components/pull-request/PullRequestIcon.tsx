import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import type { PullRequestState } from '@/bindings/PullRequestState'
import type { PullRequestProblem } from '@/lib/pull-request-problem'
import {
  GitMergeIcon,
  GitPullRequestClosedIcon,
  GitPullRequestDraftIcon,
  GitPullRequestIcon
} from '@hugeicons/core-free-icons'

export const PULL_REQUEST_STATE: Record<
  PullRequestState,
  { icon: typeof GitPullRequestIcon; color: string; label: () => string }
> = {
  open: {
    icon: GitPullRequestIcon,
    color: 'text-pr-open',
    label: m.pr_state_open
  },
  draft: {
    icon: GitPullRequestDraftIcon,
    color: 'text-muted-foreground',
    label: m.pr_state_draft
  },
  merged: {
    icon: GitMergeIcon,
    color: 'text-pr-merged',
    label: m.pr_state_merged
  },
  closed: {
    icon: GitPullRequestClosedIcon,
    color: 'text-status-error',
    label: m.pr_state_closed
  }
}

/** The merge button's colors: conflicts and failing checks block, requested changes warn. */
export const PULL_REQUEST_PROBLEM: Record<
  PullRequestProblem,
  { color: string; label: () => string }
> = {
  conflicts: { color: 'text-status-error', label: m.pr_problem_conflicts },
  checksFailing: { color: 'text-status-error', label: m.checks_failing },
  changesRequested: {
    color: 'text-warning',
    label: m.pr_problem_changes_requested
  }
}

export function PullRequestIcon({
  state,
  problem = null,
  className
}: {
  state: PullRequestState
  problem?: PullRequestProblem | null
  className?: string
}) {
  const { icon, color } = PULL_REQUEST_STATE[state]
  return (
    <HugeiconsIcon
      icon={icon}
      strokeWidth={2}
      className={cn(
        'shrink-0',
        problem ? PULL_REQUEST_PROBLEM[problem].color : color,
        className
      )}
    />
  )
}
