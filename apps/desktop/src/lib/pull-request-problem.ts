import type { PullRequest } from '@/bindings/PullRequest'

export type PullRequestProblem =
  | 'conflicts'
  | 'checksFailing'
  | 'changesRequested'

/** What holds an open pull request back, the worst first; `null` when nothing does. */
export function pullRequestProblem(
  pullRequest: PullRequest
): PullRequestProblem | null {
  if (pullRequest.state !== 'open') return null
  if (pullRequest.conflicts) return 'conflicts'
  if (pullRequest.checks === 'failing') return 'checksFailing'
  if (pullRequest.changesRequested) return 'changesRequested'
  return null
}
