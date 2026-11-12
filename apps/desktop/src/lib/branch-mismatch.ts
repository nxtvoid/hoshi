import type { Thread } from '@/bindings/Thread'

export type BranchMismatch = { threadBranch: string; rootBranch: string }

/**
 * A thread in the project root that worked on another branch than the one the root is on
 * now. Threads with a worktree have a folder of their own, so they never mismatch.
 */
export function branchMismatch(
  thread: Pick<Thread, 'branch' | 'worktreePath' | 'projectId'>,
  rootBranch: string | null | undefined
): BranchMismatch | null {
  if (thread.worktreePath !== null || thread.projectId === null) return null
  if (!thread.branch || !rootBranch || thread.branch === rootBranch) return null
  return { threadBranch: thread.branch, rootBranch }
}

/** Names one mismatch, so dismissing it doesn't hide the next one. */
export function mismatchKey(threadId: string, mismatch: BranchMismatch) {
  return `${threadId}:${mismatch.threadBranch}:${mismatch.rootBranch}`
}
