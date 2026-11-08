import type { Thread } from '@/bindings/Thread'
import type { PullRequest } from '@/bindings/PullRequest'

/** A worktree to remove, or a project root to switch back to the base branch. */
export type MergedCleanup = 'worktree' | 'root'

/** What cleaning up the thread would do once its pull request is merged; `null` when nothing. */
export function mergedCleanup(
  thread: Pick<Thread, 'worktreePath' | 'branch'>,
  pullRequest: PullRequest | undefined
): MergedCleanup | null {
  if (pullRequest?.state !== 'merged') return null
  if (thread.worktreePath !== null) return 'worktree'
  const onHead =
    thread.branch === pullRequest.headBranch &&
    pullRequest.headBranch !== pullRequest.baseBranch
  return onHead ? 'root' : null
}
