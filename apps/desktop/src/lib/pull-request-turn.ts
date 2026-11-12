import type { ReviewTurn } from '@/bindings/ReviewTurn'
import type { PullRequest } from '@/bindings/PullRequest'

export type PullRequestTurn =
  | {
      kind: 'author'
      /** What was asked of the author, when the forge told who and when. */
      ask: Extract<ReviewTurn, { kind: 'author' }> | null
      threads: number
      tasks: number
    }
  | {
      kind: 'reviewers'
      waitingOn: string[]
      repliedAt: string
      tasks: number
    }

/** Whose move an open pull request waits on, as its author sees it; `null` when nobody's. */
export function pullRequestTurn(
  pullRequest: PullRequest
): PullRequestTurn | null {
  const { turn, unresolvedComments, openTasks, changesRequested } = pullRequest
  if (pullRequest.state !== 'open' && pullRequest.state !== 'draft') return null
  const ask = turn?.kind === 'author' ? turn : null
  const unanswered = turn === null ? changesRequested : ask !== null
  if (unanswered || unresolvedComments > 0) {
    return {
      kind: 'author',
      ask,
      threads: unresolvedComments,
      tasks: openTasks
    }
  }
  if (turn?.kind === 'reviewers') {
    return {
      kind: 'reviewers',
      waitingOn: turn.waitingOn,
      repliedAt: turn.repliedAt,
      tasks: openTasks
    }
  }
  return null
}
