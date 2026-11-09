import { cn } from '@hoshi/utils'
import { useState } from 'react'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import type { PullRequestUpdate } from '@/bindings/PullRequestUpdate'
import {
  AlertCircleIcon,
  ArrowRight01Icon,
  ViewIcon
} from '@hugeicons/core-free-icons'

/** What changed, in a few words: "checks failed, 2 new comments". */
export function updateSummary(update: PullRequestUpdate) {
  const parts: string[] = []
  if (update.failedChecks.length > 0) parts.push(m.pr_update_checks_failed())
  if (update.checksPassed) parts.push(m.pr_update_checks_passed())
  if (update.comments > 0) {
    parts.push(m.pr_update_comments({ count: update.comments }))
  }
  if (update.conflicting) parts.push(m.pr_update_conflicting())
  if (update.closed) parts.push(m.pr_update_closed())
  if (update.stopped && !update.closed) parts.push(m.pr_update_stopped())
  return parts.join(', ')
}

/**
 * News on a pull request the thread watches, which woke its agent: one line, as t3code
 * shows it, opening to what the agent was told.
 */
export function PullRequestUpdateRow({
  update,
  text
}: {
  update: PullRequestUpdate
  text: string
}) {
  const [open, setOpen] = useState(false)
  const alarming = update.failedChecks.length > 0 || update.conflicting
  return (
    <div className='flex flex-col gap-1.5'>
      <button
        type='button'
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className='text-muted-foreground hover:text-foreground focus-visible:ring-ring flex min-h-6 items-center gap-2 rounded-md text-left text-sm outline-none focus-visible:ring-2'
      >
        <HugeiconsIcon
          icon={alarming ? AlertCircleIcon : ViewIcon}
          strokeWidth={2}
          className={cn('size-4 shrink-0', alarming && 'text-status-error')}
        />
        <span className='text-foreground shrink-0 font-medium tabular-nums'>
          #{update.number}
        </span>
        <span className='min-w-0 truncate'>{updateSummary(update)}</span>
        <HugeiconsIcon
          icon={ArrowRight01Icon}
          strokeWidth={2}
          className={cn(
            'size-3.5 shrink-0 transition-transform',
            open && 'rotate-90'
          )}
        />
      </button>
      {open ? (
        <p className='text-muted-foreground border-border ms-2 border-s ps-4 text-sm whitespace-pre-wrap'>
          {text}
        </p>
      ) : null}
    </div>
  )
}
