import { useState } from 'react'
import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { openUrl } from '@tauri-apps/plugin-opener'
import { confirm } from '@/state/dialog-store'
import { Input } from '@hoshi/ui/components/input'
import { Button } from '@hoshi/ui/components/button'
import { relativeTime } from './shared'
import type { MergeMethod } from '@/bindings/MergeMethod'
import type { PullRequest } from '@/bindings/PullRequest'
import type { PullRequestActions } from './use-pull-request-actions'
import type { PullRequestDetail } from '@/bindings/PullRequestDetail'
import { PullRequestIcon } from '@/components/pull-request/PullRequestIcon'
import {
  ArrowDown01Icon,
  CancelCircleIcon,
  Copy01Icon,
  File01Icon,
  GitMergeConflictIcon,
  GitMergeIcon,
  GitPullRequestClosedIcon,
  GitPullRequestDraftIcon,
  LinkSquare02Icon,
  Loading03Icon,
  LockIcon,
  MoreHorizontalIcon,
  PencilEdit02Icon,
  Tick02Icon
} from '@hugeicons/core-free-icons'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger
} from '@hoshi/ui/components/dropdown-menu'

const MERGE_METHOD_LABEL: Record<MergeMethod, () => string> = {
  merge: m.merge_method_merge,
  squash: m.merge_method_squash,
  rebase: m.merge_method_rebase
}

function checkoutCommand(pullRequest: PullRequest) {
  switch (pullRequest.forge) {
    case 'github':
      return `gh pr checkout ${pullRequest.number}`
    case 'bitbucket':
      return `git fetch origin ${pullRequest.headBranch} && git switch ${pullRequest.headBranch}`
  }
}

export function PullRequestHeader({
  pullRequest,
  detail,
  actions
}: {
  pullRequest: PullRequest
  detail: PullRequestDetail | null
  actions: PullRequestActions
}) {
  const checkout = checkoutCommand(pullRequest)

  return (
    <section className='border-border flex shrink-0 flex-col gap-1.5 border-b px-3 pt-3 pb-2.5'>
      <div className='flex items-center gap-1.5'>
        <button
          type='button'
          onClick={() => void openUrl(pullRequest.url)}
          className='text-muted-foreground hover:text-foreground focus-visible:ring-ring flex min-w-0 items-center gap-1 rounded text-xs outline-none focus-visible:ring-2'
        >
          {detail ? (
            <span className='truncate'>{detail.repository}</span>
          ) : null}
          <span className='text-foreground font-medium tabular-nums'>
            #{pullRequest.number}
          </span>
          <HugeiconsIcon
            icon={LinkSquare02Icon}
            strokeWidth={2}
            className='size-3 shrink-0'
          />
        </button>
        <div className='ml-auto flex items-center gap-1'>
          {detail ? (
            <MergeMenu
              pullRequest={pullRequest}
              detail={detail}
              actions={actions}
            />
          ) : null}
          <PullRequestMenu
            pullRequest={pullRequest}
            detail={detail}
            actions={actions}
          />
        </div>
      </div>
      <Title pullRequest={pullRequest} detail={detail} actions={actions} />
      <div className='text-muted-foreground flex items-center gap-2 text-xs'>
        <PullRequestIcon state={pullRequest.state} className='size-3.5' />
        <span className='min-w-0 flex-1 truncate'>
          {pullRequest.author ? (
            <span className='text-foreground font-medium'>
              {pullRequest.author}
            </span>
          ) : null}
          {pullRequest.author ? ' · ' : null}
          {m.pr_updated({ time: relativeTime(pullRequest.updatedAt) })}
        </span>
      </div>
      <div className='text-muted-foreground flex items-center gap-2 text-xs'>
        <span className='flex min-w-0 flex-1 items-center gap-1 font-mono'>
          <span className='shrink-0'>{pullRequest.baseBranch} ←</span>
          <span className='text-foreground/85 truncate'>
            {pullRequest.headBranch}
          </span>
          {checkout ? <CopyCheckout command={checkout} /> : null}
        </span>
        <span className='flex shrink-0 items-center gap-1 tabular-nums'>
          <HugeiconsIcon icon={File01Icon} strokeWidth={2} className='size-3' />
          {detail ? m.diff_file_count({ count: detail.changedFiles }) : null}
          <span className='text-pr-open'>+{pullRequest.additions}</span>
          <span className='text-status-error'>−{pullRequest.deletions}</span>
        </span>
      </div>
    </section>
  )
}

function Title({
  pullRequest,
  detail,
  actions
}: {
  pullRequest: PullRequest
  detail: PullRequestDetail | null
  actions: PullRequestActions
}) {
  const [editing, setEditing] = useState(false)
  const [title, setTitle] = useState(pullRequest.title)

  if (editing && detail) {
    const save = async () => {
      const next = title.trim()
      if (!next || next === pullRequest.title) {
        setEditing(false)
        return
      }
      const saved = await actions.run({
        type: 'edit',
        title: next,
        body: detail.body
      })
      if (saved) setEditing(false)
    }
    return (
      <Input
        autoFocus
        value={title}
        aria-label={m.pr_title()}
        disabled={actions.pending}
        onChange={(event) => setTitle(event.currentTarget.value)}
        onBlur={() => void save()}
        onKeyDown={(event) => {
          if (event.key === 'Enter') event.currentTarget.blur()
          if (event.key === 'Escape') {
            setTitle(pullRequest.title)
            setEditing(false)
          }
        }}
        className='h-8 text-base font-semibold'
      />
    )
  }
  return (
    <div className='group flex items-start gap-1'>
      <h2 className='min-w-0 flex-1 text-base leading-snug font-semibold text-pretty'>
        {pullRequest.title}
      </h2>
      {detail?.canEdit ? (
        <button
          type='button'
          aria-label={m.edit_title()}
          onClick={() => {
            setTitle(pullRequest.title)
            setEditing(true)
          }}
          className='text-muted-foreground hover:text-foreground focus-visible:ring-ring mt-0.5 rounded p-0.5 opacity-0 outline-none group-hover:opacity-100 focus-visible:opacity-100 focus-visible:ring-2'
        >
          <HugeiconsIcon
            icon={PencilEdit02Icon}
            strokeWidth={2}
            className='size-3.5'
          />
        </button>
      ) : null}
    </div>
  )
}

type MergeBlocker = 'permission' | 'draft' | 'conflicts'

const MERGE_BLOCKER_LABEL: Record<MergeBlocker, () => string> = {
  permission: m.merge_blocked_permission,
  draft: m.merge_blocked_draft,
  conflicts: m.merge_blocked_conflicts
}

const MERGE_BLOCKER_ICON: Record<MergeBlocker, typeof GitMergeIcon> = {
  permission: LockIcon,
  draft: GitPullRequestDraftIcon,
  conflicts: GitMergeConflictIcon
}

/** Why the pull request can't be merged now, or `null` when it can. */
function mergeBlocker(
  pullRequest: PullRequest,
  detail: PullRequestDetail
): MergeBlocker | null {
  if (!detail.canEdit) return 'permission'
  if (pullRequest.state === 'draft') return 'draft'
  if (detail.mergeability === 'conflicting') return 'conflicts'
  return null
}

function MergeMenu({
  pullRequest,
  detail,
  actions
}: {
  pullRequest: PullRequest
  detail: PullRequestDetail
  actions: PullRequestActions
}) {
  const finished =
    pullRequest.state === 'merged' || pullRequest.state === 'closed'
  if (finished || detail.mergeMethods.length === 0) return null
  const blocker = mergeBlocker(pullRequest, detail)
  const changesRequested = detail.reviewers
    .filter((reviewer) => reviewer.state === 'changesRequested')
    .map((reviewer) => reviewer.name)
  const warning =
    changesRequested.length > 0
      ? m.merge_changes_requested({ names: changesRequested.join(', ') })
      : null
  const conflicts = blocker === 'conflicts'
  const hint = blocker ? MERGE_BLOCKER_LABEL[blocker]() : warning

  const merge = async (method: MergeMethod) => {
    const question = m.merge_confirm({
      number: pullRequest.number,
      base: pullRequest.baseBranch,
      method: MERGE_METHOD_LABEL[method]()
    })
    const confirmed = await confirm({
      title: MERGE_METHOD_LABEL[method](),
      description: warning ? `${warning}. ${question}` : question,
      confirmLabel: m.merge()
    })
    if (confirmed) await actions.run({ type: 'merge', method })
  }

  const trigger = (
    <DropdownMenuTrigger
      disabled={blocker !== null || actions.pending}
      render={
        conflicts ? (
          <Button
            size='xs'
            variant='destructive'
            className='disabled:opacity-100'
          />
        ) : warning ? (
          <Button
            size='xs'
            variant='secondary'
            className='bg-warning-surface text-warning hover:bg-warning/20'
          />
        ) : (
          <Button size='xs' className='bg-pr-open hover:bg-pr-open/90' />
        )
      }
    >
      {actions.pending ? (
        <HugeiconsIcon
          icon={Loading03Icon}
          strokeWidth={2}
          className='animate-spin'
        />
      ) : blocker ? (
        <HugeiconsIcon icon={MERGE_BLOCKER_ICON[blocker]} strokeWidth={2} />
      ) : warning ? (
        <HugeiconsIcon icon={CancelCircleIcon} strokeWidth={2} />
      ) : (
        <HugeiconsIcon icon={GitMergeIcon} strokeWidth={2} />
      )}
      {m.merge()}
      <HugeiconsIcon icon={ArrowDown01Icon} strokeWidth={2} />
    </DropdownMenuTrigger>
  )

  return (
    <DropdownMenu>
      {hint ? (
        <Tooltip>
          <TooltipTrigger render={<span />}>{trigger}</TooltipTrigger>
          <TooltipContent side='bottom'>{hint}</TooltipContent>
        </Tooltip>
      ) : (
        trigger
      )}
      <DropdownMenuContent align='end' className='w-48'>
        {detail.mergeMethods.map((method) => (
          <DropdownMenuItem key={method} onClick={() => void merge(method)}>
            {MERGE_METHOD_LABEL[method]()}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function PullRequestMenu({
  pullRequest,
  detail,
  actions
}: {
  pullRequest: PullRequest
  detail: PullRequestDetail | null
  actions: PullRequestActions
}) {
  const checkout = checkoutCommand(pullRequest)
  const open = pullRequest.state === 'open' || pullRequest.state === 'draft'
  const canEdit = detail?.canEdit === true && pullRequest.state !== 'merged'
  const canToggleDraft = open && detail?.supports.draft === true
  const canClose = open || detail?.supports.reopen === true

  const setOpen = async (next: boolean) => {
    if (!next) {
      const confirmed = await confirm({
        title: m.close_pull_request(),
        description: m.close_pull_request_confirm({
          number: pullRequest.number
        }),
        confirmLabel: m.close_pull_request(),
        tone: 'destructive',
        icon: GitPullRequestClosedIcon
      })
      if (!confirmed) return
    }
    await actions.run({ type: 'setOpen', open: next })
  }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            variant='ghost'
            size='icon-xs'
            aria-label={m.pull_request_actions()}
          />
        }
      >
        <HugeiconsIcon icon={MoreHorizontalIcon} strokeWidth={2} />
      </DropdownMenuTrigger>
      <DropdownMenuContent align='end' className='w-52'>
        <DropdownMenuItem onClick={() => void openUrl(pullRequest.url)}>
          {m.open_in_browser()}
        </DropdownMenuItem>
        <DropdownMenuItem
          onClick={() => void navigator.clipboard.writeText(pullRequest.url)}
        >
          {m.copy_link()}
        </DropdownMenuItem>
        {checkout ? (
          <DropdownMenuItem
            onClick={() => void navigator.clipboard.writeText(checkout)}
          >
            {m.copy_checkout_command()}
          </DropdownMenuItem>
        ) : null}
        {canEdit ? (
          <>
            {canToggleDraft || canClose ? <DropdownMenuSeparator /> : null}
            {canToggleDraft ? (
              <DropdownMenuItem
                disabled={actions.pending}
                onClick={() =>
                  void actions.run({
                    type: 'setDraft',
                    draft: pullRequest.state !== 'draft'
                  })
                }
              >
                {pullRequest.state === 'draft'
                  ? m.mark_ready_for_review()
                  : m.convert_to_draft()}
              </DropdownMenuItem>
            ) : null}
            {canClose ? (
              <DropdownMenuItem
                variant={open ? 'destructive' : 'default'}
                disabled={actions.pending}
                onClick={() => void setOpen(!open)}
              >
                {open ? m.close_pull_request() : m.reopen_pull_request()}
              </DropdownMenuItem>
            ) : null}
          </>
        ) : null}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function CopyCheckout({ command }: { command: string }) {
  const [copied, setCopied] = useState(false)

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <button
            type='button'
            aria-label={m.copy_checkout_command()}
            onClick={() => {
              void navigator.clipboard.writeText(command).then(() => {
                setCopied(true)
                setTimeout(() => setCopied(false), 1500)
              })
            }}
            className='hover:bg-accent hover:text-foreground focus-visible:ring-ring flex size-5 shrink-0 items-center justify-center rounded outline-none focus-visible:ring-2'
          />
        }
      >
        <HugeiconsIcon
          icon={copied ? Tick02Icon : Copy01Icon}
          strokeWidth={2}
          className={cn('size-3', copied && 'text-pr-open')}
        />
      </TooltipTrigger>
      <TooltipContent side='bottom' sideOffset={8} className='font-mono'>
        {command}
      </TooltipContent>
    </Tooltip>
  )
}
