import { m } from '@/paraglide/messages'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@hoshi/ui/components/button'
import { useCleanupStore } from '@/state/cleanup-store'
import { mergedCleanup } from '@/state/merged-cleanup'
import { GitMergeIcon } from '@hugeicons/core-free-icons'
import {
  cleanupErrorMessage,
  useWorkspaceCleanup
} from './use-workspace-cleanup'

/**
 * Once a thread's pull request is merged, offers to remove its worktree and local branch, or
 * to switch the project root back to the base branch and delete the merged one.
 * Everything it removes is on this machine; the forge keeps its own branch.
 */
export function MergedCleanupBanner({ threadId }: { threadId: string }) {
  const thread = useAppStore((state) =>
    state.threads.find((candidate) => candidate.id === threadId)
  )
  const pullRequest = useAppStore((state) => state.pullRequests[threadId])
  const dismissed = useCleanupStore((state) => state.dismissed[threadId])
  const dismiss = useCleanupStore((state) => state.dismiss)
  const cleanup = useWorkspaceCleanup(threadId)
  const kind = thread ? mergedCleanup(thread, pullRequest) : null

  if (
    !thread ||
    !pullRequest ||
    !kind ||
    dismissed ||
    thread.status === 'working'
  ) {
    return null
  }
  const unpushed = cleanup.error?.code === 'unpushedCommits'

  return (
    <div
      role='status'
      className='border-border bg-muted/40 flex shrink-0 flex-col gap-1.5 border-b px-4 py-2.5 text-sm'
    >
      <div className='flex items-center gap-3'>
        <HugeiconsIcon
          icon={GitMergeIcon}
          strokeWidth={2}
          className='text-pr-merged size-4 shrink-0'
        />
        <p className='min-w-0 flex-1 text-pretty'>
          {kind === 'root'
            ? m.cleanup_offer_root({
                number: pullRequest.number,
                branch: pullRequest.headBranch,
                base: pullRequest.baseBranch
              })
            : m.cleanup_offer({
                number: pullRequest.number,
                branch: thread.branch ?? ''
              })}
          <span className='text-muted-foreground'>
            {' '}
            {m.cleanup_local_only()}
          </span>
        </p>
        <div className='flex shrink-0 items-center gap-1.5'>
          <Button
            variant='ghost'
            size='sm'
            disabled={cleanup.pending !== null}
            onClick={() => dismiss(threadId)}
          >
            {m.cleanup_keep()}
          </Button>
          {unpushed && kind === 'worktree' && !thread.worktreeReleased ? (
            <Button
              variant='outline'
              size='sm'
              disabled={cleanup.pending !== null}
              onClick={() => void cleanup.run('release')}
            >
              {m.release_worktree()}
            </Button>
          ) : null}
          <Button
            size='sm'
            disabled={cleanup.pending !== null}
            onClick={() => void cleanup.run('cleanUp')}
          >
            {cleanup.pending === 'cleanUp'
              ? m.cleanup_running()
              : m.clean_up_thread()}
          </Button>
        </div>
      </div>
      {cleanup.error ? (
        <p role='alert' className='text-destructive pl-7 text-xs'>
          {cleanupErrorMessage(cleanup.error)}
        </p>
      ) : null}
    </div>
  )
}
