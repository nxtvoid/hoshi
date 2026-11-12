import { useState } from 'react'
import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { CHAT_LANE } from '@/lib/chat-lane'
import { invoke } from '@tauri-apps/api/core'
import { HugeiconsIcon } from '@hugeicons/react'
import { useAppStore } from '@/state/app-store'
import { Button } from '@hoshi/ui/components/button'
import { hasDraft, useDraftStore } from '@/state/draft-store'
import type { CommandError } from '@/bindings/CommandError'
import { useProjectBranchStore } from '@/state/project-branch-store'
import { AlertCircleIcon, Cancel01Icon } from '@hugeicons/core-free-icons'
import { branchMismatch, mismatchKey } from '@/lib/branch-mismatch'

/**
 * Above the composer once the user starts writing in a thread of the project root whose
 * branch the root no longer is on: offers to switch the root back to it.
 */
export function BranchMismatchBar({ threadId }: { threadId: string }) {
  const branch = useAppStore(
    (state) => state.threads.find((thread) => thread.id === threadId)?.branch
  )
  const worktreePath = useAppStore(
    (state) =>
      state.threads.find((thread) => thread.id === threadId)?.worktreePath
  )
  const projectId = useAppStore(
    (state) => state.threads.find((thread) => thread.id === threadId)?.projectId
  )
  const working = useAppStore(
    (state) =>
      state.threads.find((thread) => thread.id === threadId)?.status ===
      'working'
  )
  const rootBranch = useProjectBranchStore((state) =>
    projectId ? state.byProject[projectId] : undefined
  )
  const writing = useDraftStore((state) => hasDraft(state, threadId))
  const dismiss = useProjectBranchStore((state) => state.dismiss)
  const [shownKey, setShownKey] = useState<string | null>(null)
  const [switching, setSwitching] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const mismatch =
    branch === undefined ||
    worktreePath === undefined ||
    projectId === undefined
      ? null
      : branchMismatch({ branch, worktreePath, projectId }, rootBranch)
  const key = mismatch ? mismatchKey(threadId, mismatch) : null
  const dismissed = useProjectBranchStore(
    (state) => key !== null && state.dismissed[key] === true
  )
  if (key && writing && shownKey !== key) setShownKey(key)
  if (!mismatch || !key || dismissed || working) return null
  if (!writing && shownKey !== key) return null

  const switchBack = async () => {
    setSwitching(true)
    setError(null)
    const failure = await invoke('switch_branch', {
      threadId,
      branch: mismatch.threadBranch
    }).then(
      () => null,
      (failure: CommandError) => failure
    )
    setSwitching(false)
    if (failure) {
      setError(
        failure.code === 'gitFailed' ? failure.detail : m.error_generic()
      )
    }
  }

  return (
    <div className={cn(CHAT_LANE, 'mb-2 flex flex-col gap-1')}>
      <div className='border-warning/30 bg-warning-surface flex items-center gap-2 rounded-lg border py-1.5 ps-3 pe-1.5 text-xs'>
        <HugeiconsIcon
          icon={AlertCircleIcon}
          strokeWidth={2}
          className='text-warning size-3.5 shrink-0'
        />
        <span className='min-w-0 flex-1'>
          {m.branch_mismatch({
            threadBranch: mismatch.threadBranch,
            rootBranch: mismatch.rootBranch
          })}
        </span>
        <Button
          size='xs'
          variant='outline'
          disabled={switching}
          onClick={() => void switchBack()}
        >
          {m.switch_to_branch({ branch: mismatch.threadBranch })}
        </Button>
        <Button
          size='icon-xs'
          variant='ghost'
          aria-label={m.dismiss()}
          className='text-muted-foreground hover:text-foreground'
          onClick={() => dismiss(key)}
        >
          <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} />
        </Button>
      </div>
      {error ? (
        <p className='text-status-error px-1 text-xs break-words'>{error}</p>
      ) : null}
    </div>
  )
}
