import { memo } from 'react'
import { m } from '@/paraglide/messages'
import { useDraftStore } from '@/state/draft-store'
import { getLocale } from '@/paraglide/runtime'
import type { Thread } from '@/bindings/Thread'
import { useAppStore } from '@/state/app-store'
import { usePanelStore } from '@/state/panel-store'
import { useUnseenStore } from '@/state/unseen-store'
import { mergedCleanup } from '@/state/merged-cleanup'
import { runCleanupWithDialog } from '@/components/thread/use-workspace-cleanup'
import { HugeiconsIcon } from '@hugeicons/react'
import { cn, formatRelativeTime } from '@hoshi/utils'
import type { ThreadStatus } from '@/bindings/ThreadStatus'
import { ClaudeIcon } from '@/components/brand/ProviderIcon'
import { PullRequestIcon } from '@/components/pull-request/PullRequestIcon'
import { PullRequestHoverCard } from '@/components/pull-request/PullRequestHoverCard'
import { pullRequestPillHover, ThreadCardTrigger } from './ThreadCard'
import {
  Comment01Icon,
  GitBranchIcon,
  GitForkIcon
} from '@hugeicons/core-free-icons'
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger
} from '@hoshi/ui/components/context-menu'

const STATUS: Record<
  Exclude<ThreadStatus, 'idle'>,
  { label: () => string; dot: string; text: string }
> = {
  working: {
    label: m.status_working,
    dot: 'bg-status-working',
    text: 'text-status-working'
  },
  waitingForInput: {
    label: m.status_waiting_for_input,
    dot: 'bg-status-waiting',
    text: 'text-status-waiting'
  },
  error: {
    label: m.status_error,
    dot: 'bg-status-error',
    text: 'text-status-error'
  }
}

/** A turn that finished while the user looked elsewhere, until they open the thread. */
const COMPLETED = {
  label: m.status_completed,
  dot: 'bg-status-done',
  text: 'text-status-done'
}

type ThreadRowProps = {
  thread: Thread
  selected: boolean
  onSelect: (threadId: string) => void
  /** Names the project too, in lists that mix projects. */
  showProject?: boolean
}

export const ThreadRow = memo(function ThreadRow({
  thread,
  selected,
  onSelect,
  showProject = false
}: ThreadRowProps) {
  const deleteThread = useAppStore((state) => state.deleteThread)
  const archiveThread = useAppStore((state) => state.archiveThread)
  const pinThread = useAppStore((state) => state.pinThread)
  const regenerateTitle = useAppStore((state) => state.regenerateTitle)
  const openSurface = usePanelStore((state) => state.openSurface)
  const openPullRequest = () => {
    onSelect(thread.id)
    openSurface(thread.id, 'pullRequest')
  }
  const pullRequest = useAppStore((state) => state.pullRequests[thread.id])
  const projectTitle = useAppStore((state) =>
    !showProject
      ? undefined
      : thread.projectId === null
        ? m.chat()
        : state.projects.find((project) => project.id === thread.projectId)
            ?.title
  )
  const unseen = useUnseenStore((state) => state.unseen[thread.id] === true)
  const draft = useDraftStore((state) => thread.id in state.drafts)
  const saveDraft = useDraftStore((state) => state.save)
  const status =
    thread.status !== 'idle' ? STATUS[thread.status] : unseen ? COMPLETED : null

  return (
    <ContextMenu>
      <ThreadCardTrigger
        threadId={thread.id}
        render={
          <ContextMenuTrigger
            render={
              <button
                type='button'
                aria-current={selected || undefined}
                onClick={() => onSelect(thread.id)}
                className={cn(
                  'hover:bg-sidebar-accent/60 focus-visible:ring-sidebar-ring flex w-full flex-col gap-0.5 rounded-md px-2 py-1.5 text-left outline-none focus-visible:ring-2',
                  selected && 'bg-sidebar-accent text-sidebar-accent-foreground'
                )}
              />
            }
          />
        }
      >
        <span className='flex min-w-0 items-center gap-2'>
          <ClaudeIcon className='text-provider-claude size-3.5 shrink-0' />
          <span
            className={cn(
              'min-w-0 flex-1 truncate text-sm',
              thread.archived && 'text-muted-foreground'
            )}
          >
            {thread.title || m.untitled_thread()}
          </span>
          {status ? (
            <span
              className={cn(
                'flex shrink-0 items-center gap-1.5 text-xs',
                status.text
              )}
            >
              <span className={cn('size-1.5 rounded-full', status.dot)} />
              {status.label()}
            </span>
          ) : draft && !selected ? (
            <span className='text-draft shrink-0 text-xs italic'>
              {m.draft()}
            </span>
          ) : (
            <span className='text-muted-foreground shrink-0 text-xs tabular-nums'>
              {formatRelativeTime(thread.updatedAt, new Date(), getLocale())}
            </span>
          )}
        </span>
        {thread.branch || projectTitle ? (
          <span className='text-muted-foreground flex min-w-0 items-center gap-1.5 text-xs'>
            {projectTitle ? (
              <span className='text-sidebar-foreground/80 max-w-[45%] shrink-0 truncate'>
                {projectTitle}
              </span>
            ) : null}
            {thread.branch ? (
              <>
                <HugeiconsIcon
                  icon={thread.worktreePath ? GitForkIcon : GitBranchIcon}
                  strokeWidth={2}
                  aria-label={
                    thread.worktreeReleased
                      ? m.worktree_released_hint()
                      : undefined
                  }
                  className={cn(
                    'size-3 shrink-0',
                    thread.worktreeReleased && 'opacity-40'
                  )}
                />
                <span className='truncate font-mono text-[0.6875rem]'>
                  {thread.branch}
                </span>
              </>
            ) : null}
            {pullRequest ? (
              <PullRequestHoverCard
                threadId={thread.id}
                pullRequest={pullRequest}
                side='right'
                trigger={
                  // A real button can't sit inside the row's button.
                  // biome-ignore lint/a11y/useSemanticElements: see above
                  <span
                    role='button'
                    tabIndex={0}
                    aria-label={m.open_pull_request({
                      number: pullRequest.number
                    })}
                    onClick={(event) => {
                      event.stopPropagation()
                      openPullRequest()
                    }}
                    {...pullRequestPillHover}
                    onKeyDown={(event) => {
                      if (event.key !== 'Enter' && event.key !== ' ') return
                      event.preventDefault()
                      event.stopPropagation()
                      openPullRequest()
                    }}
                    className='hover:text-foreground hover:bg-sidebar-accent focus-visible:ring-sidebar-ring -my-0.5 -mr-1 ml-auto flex shrink-0 cursor-pointer items-center gap-1 rounded px-1 py-0.5 outline-none focus-visible:ring-2'
                  >
                    <PullRequestIcon
                      state={pullRequest.state}
                      className='size-3'
                    />
                    <span className='tabular-nums'>#{pullRequest.number}</span>
                    {pullRequest.checks === 'failing' ? (
                      <span
                        role='img'
                        aria-label={m.checks_failing()}
                        className='bg-status-error size-1.5 rounded-full'
                      />
                    ) : null}
                    {pullRequest.unresolvedComments > 0 ? (
                      <span
                        role='img'
                        className='flex items-center gap-0.5'
                        aria-label={m.unresolved_comments({
                          count: pullRequest.unresolvedComments
                        })}
                      >
                        <HugeiconsIcon
                          icon={Comment01Icon}
                          strokeWidth={2}
                          className='size-3'
                        />
                        <span className='tabular-nums'>
                          {pullRequest.unresolvedComments}
                        </span>
                      </span>
                    ) : null}
                  </span>
                }
              />
            ) : null}
          </span>
        ) : null}
      </ThreadCardTrigger>
      <ContextMenuContent>
        <ContextMenuItem onClick={() => pinThread(thread.id, !thread.pinned)}>
          {thread.pinned ? m.unpin_thread() : m.pin_thread()}
        </ContextMenuItem>
        <ContextMenuItem
          onClick={() => archiveThread(thread.id, !thread.archived)}
        >
          {thread.archived ? m.unarchive_thread() : m.archive_thread()}
        </ContextMenuItem>
        <ContextMenuItem onClick={() => regenerateTitle(thread.id)}>
          {m.regenerate_title()}
        </ContextMenuItem>
        {draft && !selected ? (
          <ContextMenuItem onClick={() => saveDraft(thread.id, null)}>
            {m.discard_draft()}
          </ContextMenuItem>
        ) : null}
        {thread.worktreePath || mergedCleanup(thread, pullRequest) ? (
          <>
            <ContextMenuSeparator />
            {mergedCleanup(thread, pullRequest) ? (
              <ContextMenuItem
                onClick={() => void runCleanupWithDialog('cleanUp', thread.id)}
              >
                {m.clean_up_thread()}
              </ContextMenuItem>
            ) : null}
            {!thread.worktreePath || thread.worktreeReleased ? null : (
              <ContextMenuItem
                onClick={() => void runCleanupWithDialog('release', thread.id)}
              >
                {m.release_worktree()}
              </ContextMenuItem>
            )}
          </>
        ) : null}
        <ContextMenuSeparator />
        <ContextMenuItem
          variant='destructive'
          onClick={() => deleteThread(thread.id)}
        >
          {m.delete_thread()}
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
})
