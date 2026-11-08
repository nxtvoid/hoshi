import { m } from '@/paraglide/messages'
import { useAppStore } from '@/state/app-store'
import { CopyButton } from '@hoshi/ui/components/copy-button'
import { GitBranchIcon, GitForkIcon } from '@hugeicons/core-free-icons'
import { PullRequestPill } from '@/components/pull-request/PullRequestPill'

const CHIP_SEGMENT =
  'flex h-full min-w-0 cursor-pointer items-center gap-1 rounded-none border-0 px-2 outline-none first:rounded-s-full last:rounded-e-full hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset'

/** The thread's branch and pull request in the chat header, as one chip. */
export function ThreadGitChip({ threadId }: { threadId: string }) {
  const branch = useAppStore(
    (state) =>
      state.threads.find((thread) => thread.id === threadId)?.branch ?? null
  )
  const worktree = useAppStore((state) =>
    Boolean(
      state.threads.find((thread) => thread.id === threadId)?.worktreePath
    )
  )
  const hasPullRequest = useAppStore(
    (state) => state.pullRequests[threadId] !== undefined
  )
  if (!branch && !hasPullRequest) return null

  return (
    <div className='border-border flex h-6 shrink-0 items-center rounded-full border text-xs'>
      {branch ? (
        <CopyButton
          value={branch}
          label={m.copy_branch({ branch })}
          copiedLabel={m.copied()}
          icon={worktree ? GitForkIcon : GitBranchIcon}
          iconClassName='text-muted-foreground'
          className={CHIP_SEGMENT}
        >
          <span className='text-muted-foreground max-w-48 truncate'>
            {branch}
          </span>
        </CopyButton>
      ) : null}
      {branch && hasPullRequest ? (
        <span aria-hidden='true' className='bg-border h-3 w-px shrink-0' />
      ) : null}
      <PullRequestPill threadId={threadId} className={CHIP_SEGMENT} />
    </div>
  )
}
