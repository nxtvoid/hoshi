import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { invoke } from '@tauri-apps/api/core'
import { HugeiconsIcon } from '@hugeicons/react'
import { choose, notify } from '@/state/dialog-store'
import type { GitAction } from '@/bindings/GitAction'
import type { GitStatus } from '@/bindings/GitStatus'
import type { DiffFile } from '@/bindings/DiffFile'
import { FileIcon } from '@/components/files/FileIcon'
import { Button } from '@hoshi/ui/components/button'
import { useEffect, useId, useMemo, useState } from 'react'
import type { CommandError } from '@/bindings/CommandError'
import { Textarea } from '@hoshi/ui/components/textarea'
import type { GitActionPhase } from '@/bindings/GitActionPhase'
import type { GitActionResult } from '@/bindings/GitActionResult'
import { useGitActionStore } from '@/state/git-action-store'
import type { GitActionRequest } from '@/bindings/GitActionRequest'
import { ICON, SPLIT_PRIMARY, SplitRow } from './details-rows'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger
} from '@hoshi/ui/components/tooltip'
import {
  DropdownMenuCheckboxItem,
  DropdownMenuItem,
  DropdownMenuSeparator
} from '@hoshi/ui/components/dropdown-menu'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogTitle
} from '@hoshi/ui/components/dialog'
import {
  CheckmarkCircle02Icon,
  CloudDownloadIcon,
  CloudUploadIcon,
  GitCommitIcon,
  GitPullRequestIcon,
  Loading03Icon
} from '@hugeicons/core-free-icons'
import {
  type GitActionLabel,
  type GitBlocker,
  menuBlockers,
  onDefaultBranch,
  pushesToDefaultBranch,
  quickAction
} from '@/lib/git-actions'

function labelText(label: GitActionLabel, draft: boolean) {
  switch (label) {
    case 'commit':
      return m.git_commit()
    case 'commitPush':
      return m.git_commit_push()
    case 'commitPushPullRequest':
      return draft ? m.git_commit_push_draft_pr() : m.git_commit_push_pr()
    case 'push':
      return m.git_push()
    case 'pushCreatePullRequest':
      return draft ? m.git_push_create_draft_pr() : m.git_push_create_pr()
    case 'createPullRequest':
      return draft ? m.git_create_draft_pr() : m.git_create_pr()
    case 'pull':
      return m.git_pull()
  }
}

function labelIcon(label: GitActionLabel) {
  switch (label) {
    case 'commit':
      return GitCommitIcon
    case 'commitPush':
    case 'push':
      return CloudUploadIcon
    case 'pull':
      return CloudDownloadIcon
    default:
      return GitPullRequestIcon
  }
}

function blockerText(blocker: GitBlocker) {
  switch (blocker) {
    case 'detached':
      return m.git_blocked_detached()
    case 'clean':
      return m.git_blocked_clean()
    case 'uncommitted':
      return m.git_blocked_uncommitted()
    case 'behind':
      return m.git_blocked_behind()
    case 'diverged':
      return m.git_blocked_diverged()
    case 'noRemote':
      return m.git_blocked_no_remote()
    case 'noCommits':
      return m.git_blocked_no_commits()
    case 'upToDate':
      return m.git_blocked_up_to_date()
  }
}

function phaseText(phase: GitActionPhase | null) {
  switch (phase) {
    case 'writingCommitMessage':
      return m.git_phase_writing_commit_message()
    case 'creatingBranch':
      return m.git_phase_creating_branch()
    case 'committing':
      return m.git_phase_committing()
    case 'pushing':
      return m.git_phase_pushing()
    case 'writingPullRequest':
      return m.git_phase_writing_pull_request()
    case 'creatingPullRequest':
      return m.git_phase_creating_pull_request()
    case 'pulling':
      return m.git_phase_pulling()
    case null:
      return m.git_phase_starting()
  }
}

function doneText(result: GitActionResult) {
  const { pullRequest, pushed, commit, subject } = result
  if (pullRequest) {
    const title = pullRequest.existing
      ? m.git_done_opened_pr({ number: pullRequest.number })
      : m.git_done_created_pr({ number: pullRequest.number })
    return { title, description: pullRequest.title }
  }
  if (pushed) {
    return {
      title: commit
        ? m.git_done_pushed({ commit, branch: pushed })
        : m.git_done_pushed_branch({ branch: pushed }),
      description: subject
    }
  }
  if (commit)
    return { title: m.git_done_committed({ commit }), description: subject }
  if (result.pulled) return { title: m.git_done_pulled(), description: null }
  return { title: m.git_done(), description: null }
}

function errorText(error: CommandError | null) {
  switch (error?.code) {
    case 'nothingToCommit':
      return m.git_error_nothing_to_commit()
    case 'commitMessageUnavailable':
      return m.git_error_commit_message()
    case 'pushBehind':
      return m.git_error_push_behind()
    case 'noRemote':
      return m.git_blocked_no_remote()
    case 'detachedHead':
      return m.git_blocked_detached()
    case 'noBaseBranch':
      return m.git_error_no_base_branch()
    case 'forgeUnauthorized':
      return m.git_error_forge_unauthorized()
    case 'forgeUnavailable':
      return m.git_error_forge_unavailable()
    case 'threadBusy':
      return m.git_error_busy()
    case 'gitFailed':
    case 'forgeRejected':
    case 'forgeUnreachable':
      return error.detail
    default:
      return m.error_generic()
  }
}

/**
 * Commit, push and pull request actions, as one button whose action follows the
 * checkout's state and a menu of the others, as t3code shows them.
 */
export function GitActionsRow({
  threadId,
  pullRequestOpen,
  working,
  changeKey
}: {
  threadId: string
  pullRequestOpen: boolean
  /** The agent works: actions wait, so they never race its own git. */
  working: boolean
  /** Changes when the checkout may have changed, to read its status again. */
  changeKey: string
}) {
  const status = useGitActionStore((state) => state.statuses[threadId])
  const running = useGitActionStore((state) => state.running[threadId])
  const done = useGitActionStore((state) => state.done[threadId])
  const draft = useGitActionStore((state) => state.draft)
  const setDraft = useGitActionStore((state) => state.setDraft)
  const loadStatus = useGitActionStore((state) => state.loadStatus)
  const run = useGitActionStore((state) => state.run)
  const [committing, setCommitting] = useState(false)

  useEffect(() => {
    if (changeKey) void loadStatus(threadId)
  }, [threadId, changeKey, loadStatus])

  if (!status) return null
  const quick = quickAction(status, pullRequestOpen)
  const blockers = menuBlockers(status)

  const start = async (request: GitActionRequest) => {
    let next = request
    if (pushesToDefaultBranch(status, request.action)) {
      const branch = status.branch ?? ''
      const answer = await choose({
        title: m.git_default_branch_title({ branch }),
        description: m.git_default_branch_description({ branch }),
        confirmLabel: m.git_default_branch_confirm({ branch }),
        alternativeLabel: m.git_default_branch_new(),
        tone: 'warning'
      })
      if (!answer) return
      next = { ...request, newBranch: answer === 'alternative' }
    }
    try {
      await run(threadId, next)
    } catch (error) {
      await notify({
        title: m.git_action_failed(),
        description: errorText(error as CommandError),
        confirmLabel: m.dialog_ok(),
        tone: 'warning'
      })
    }
  }
  const quickRequest = (action: GitAction): GitActionRequest => ({
    action,
    message: null,
    paths: null,
    newBranch: false,
    draft
  })

  return (
    <>
      <SplitRow
        primary={
          running ? (
            <Progress phase={running.phase} startedAt={running.startedAt} />
          ) : done ? (
            <Done result={done} />
          ) : (
            <QuickButton
              label={labelText(quick.label, draft)}
              icon={labelIcon(quick.label)}
              blocker={
                working
                  ? m.git_error_busy()
                  : quick.blocker && blockerText(quick.blocker)
              }
              onClick={() => void start(quickRequest(quick.action))}
            />
          )
        }
        menuLabel={m.git_action_options()}
        menu={
          running ? null : (
            <>
              <DropdownMenuItem
                disabled={working || blockers.commit !== null}
                title={
                  blockers.commit ? blockerText(blockers.commit) : undefined
                }
                onClick={() => setCommitting(true)}
              >
                <HugeiconsIcon icon={GitCommitIcon} strokeWidth={2} />
                {m.git_commit_menu()}
              </DropdownMenuItem>
              <DropdownMenuItem
                disabled={working || blockers.push !== null}
                title={blockers.push ? blockerText(blockers.push) : undefined}
                onClick={() => void start(quickRequest('push'))}
              >
                <HugeiconsIcon icon={CloudUploadIcon} strokeWidth={2} />
                {m.git_push()}
              </DropdownMenuItem>
              {pullRequestOpen ? null : (
                <DropdownMenuItem
                  disabled={working || blockers.createPullRequest !== null}
                  title={
                    blockers.createPullRequest
                      ? blockerText(blockers.createPullRequest)
                      : undefined
                  }
                  onClick={() => void start(quickRequest('createPullRequest'))}
                >
                  <HugeiconsIcon icon={GitPullRequestIcon} strokeWidth={2} />
                  {draft ? m.git_create_draft_pr() : m.git_create_pr()}
                </DropdownMenuItem>
              )}
              <DropdownMenuSeparator />
              <DropdownMenuCheckboxItem
                checked={draft}
                onCheckedChange={(checked) => setDraft(checked)}
              >
                {m.git_draft_pull_requests()}
              </DropdownMenuCheckboxItem>
            </>
          )
        }
      />
      {committing ? (
        <CommitDialog
          threadId={threadId}
          status={status}
          onClose={() => setCommitting(false)}
          onCommit={(request) => {
            setCommitting(false)
            void start(request)
          }}
        />
      ) : null}
    </>
  )
}

function QuickButton({
  label,
  icon,
  blocker,
  onClick
}: {
  label: string
  icon: typeof GitCommitIcon
  /** Why it can't run now; shown on hover. */
  blocker: string | null
  onClick: () => void
}) {
  const button = (
    <button
      type='button'
      aria-disabled={blocker !== null}
      onClick={blocker ? undefined : onClick}
      className={cn(
        SPLIT_PRIMARY,
        blocker && 'cursor-default opacity-60 hover:bg-transparent'
      )}
    >
      <HugeiconsIcon icon={icon} strokeWidth={2} className={ICON} />
      <span className='truncate'>{label}</span>
    </button>
  )
  if (!blocker) return button
  return (
    <Tooltip>
      <TooltipTrigger render={button} />
      <TooltipContent side='left' className='max-w-60'>
        {blocker}
      </TooltipContent>
    </Tooltip>
  )
}

function Progress({
  phase,
  startedAt
}: {
  phase: GitActionPhase | null
  startedAt: number
}) {
  const [now, setNow] = useState(Date.now())
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [])
  const seconds = Math.max(0, Math.round((now - startedAt) / 1000))
  const elapsed =
    seconds < 60
      ? `${seconds}s`
      : `${Math.floor(seconds / 60)}m ${seconds % 60}s`
  return (
    <div
      role='status'
      className='text-foreground/80 flex h-8 min-w-0 flex-1 items-center gap-2.5 px-2.5 text-[13px] font-medium'
    >
      <HugeiconsIcon
        icon={Loading03Icon}
        strokeWidth={2}
        className={cn(ICON, 'animate-spin')}
      />
      <span className='min-w-0 flex-1 truncate'>{phaseText(phase)}</span>
      <span className='text-muted-foreground shrink-0 text-xs tabular-nums'>
        {elapsed}
      </span>
    </div>
  )
}

function Done({ result }: { result: GitActionResult }) {
  const { title, description } = doneText(result)
  const url = result.pullRequest?.url
  const content = (
    <>
      <HugeiconsIcon
        icon={CheckmarkCircle02Icon}
        strokeWidth={2}
        className='text-status-done size-4 shrink-0'
      />
      <span className='truncate'>{title}</span>
      {description ? (
        <span className='text-muted-foreground min-w-0 flex-1 truncate font-normal'>
          {description}
        </span>
      ) : null}
    </>
  )
  return url ? (
    <a href={url} target='_blank' rel='noreferrer' className={SPLIT_PRIMARY}>
      {content}
    </a>
  ) : (
    <div
      role='status'
      className={cn(SPLIT_PRIMARY, 'cursor-default hover:bg-transparent')}
    >
      {content}
    </div>
  )
}

/** "Commit…": the files that go in, which can be left out, and the user's own message. */
function CommitDialog({
  threadId,
  status,
  onClose,
  onCommit
}: {
  threadId: string
  status: GitStatus
  onClose: () => void
  onCommit: (request: GitActionRequest) => void
}) {
  const [files, setFiles] = useState<DiffFile[] | null>(null)
  const [excluded, setExcluded] = useState<Set<string>>(new Set())
  const [message, setMessage] = useState('')
  const messageId = useId()

  useEffect(() => {
    invoke<DiffFile[]>('get_thread_diff_files', {
      threadId,
      scope: 'uncommitted'
    })
      .then(setFiles)
      .catch(() => setFiles([]))
  }, [threadId])

  const selected = useMemo(
    () => (files ?? []).filter((file) => !excluded.has(file.path)),
    [files, excluded]
  )
  const toggle = (path: string) =>
    setExcluded((current) => {
      const next = new Set(current)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      return next
    })
  const commit = (newBranch: boolean) =>
    onCommit({
      action: 'commit',
      message: message.trim() === '' ? null : message,
      paths:
        excluded.size === 0
          ? null
          : selected.flatMap((file) =>
              file.oldPath ? [file.oldPath, file.path] : [file.path]
            ),
      newBranch,
      draft: false
    })

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className='sm:max-w-lg'>
        <DialogTitle>{m.git_commit_title()}</DialogTitle>
        <DialogDescription>{m.git_commit_description()}</DialogDescription>
        <div className='flex flex-col gap-1'>
          <span className='text-muted-foreground text-xs'>
            {m.git_commit_branch()}
          </span>
          <span className='font-mono text-xs'>
            {status.branch ?? '—'}
            {onDefaultBranch(status) ? (
              <span className='text-status-waiting ms-2 font-sans'>
                {status.defaultBranch}
              </span>
            ) : null}
          </span>
        </div>
        <div className='flex min-h-0 flex-col gap-1'>
          <span className='text-muted-foreground flex justify-between text-xs'>
            {m.git_commit_files()}
            {files ? (
              <span className='tabular-nums'>
                {m.git_commit_files_count({
                  selected: selected.length,
                  total: files.length
                })}
              </span>
            ) : null}
          </span>
          <ul className='border-border max-h-56 overflow-y-auto rounded-lg border p-1'>
            {(files ?? []).map((file) => (
              <li key={file.path}>
                <label className='hover:bg-accent/60 flex h-7 cursor-pointer items-center gap-2 rounded-md px-1.5 text-xs'>
                  <input
                    type='checkbox'
                    checked={!excluded.has(file.path)}
                    onChange={() => toggle(file.path)}
                    className='accent-primary size-3.5'
                  />
                  <FileIcon path={file.path} className='size-3.5 shrink-0' />
                  <span
                    className={cn(
                      'min-w-0 flex-1 truncate font-mono',
                      excluded.has(file.path) &&
                        'text-muted-foreground line-through'
                    )}
                  >
                    {file.path}
                  </span>
                  {file.additions === null ? null : (
                    <span className='shrink-0 font-mono tabular-nums'>
                      <span className='text-pr-open'>+{file.additions}</span>{' '}
                      <span className='text-status-error'>
                        −{file.deletions ?? 0}
                      </span>
                    </span>
                  )}
                </label>
              </li>
            ))}
          </ul>
        </div>
        <div className='flex flex-col gap-1'>
          <label htmlFor={messageId} className='text-muted-foreground text-xs'>
            {m.git_commit_message()}
          </label>
          <Textarea
            id={messageId}
            value={message}
            onChange={(event) => setMessage(event.target.value)}
            placeholder={m.git_commit_message_placeholder()}
            rows={3}
          />
        </div>
        <DialogFooter>
          <Button variant='outline' onClick={onClose}>
            {m.cancel()}
          </Button>
          <Button
            variant='outline'
            disabled={selected.length === 0}
            onClick={() => commit(true)}
          >
            {m.git_commit_new_branch()}
          </Button>
          <Button
            disabled={selected.length === 0}
            onClick={() => commit(false)}
          >
            {m.git_commit()}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
