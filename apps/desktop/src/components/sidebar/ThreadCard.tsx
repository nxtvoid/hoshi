import { memo } from 'react'
import { cn } from '@/lib/utils'
import { m } from '@/paraglide/messages'
import { getLocale } from '@/paraglide/runtime'
import type { Thread } from '@/bindings/Thread'
import { HugeiconsIcon } from '@hugeicons/react'
import { formatRelativeTime } from '@/lib/format'
import { GitBranchIcon } from '@hugeicons/core-free-icons'
import type { ThreadStatus } from '@/bindings/ThreadStatus'

const STATUS: Record<
  Exclude<ThreadStatus, 'idle'>,
  { label: () => string; dot: string }
> = {
  working: { label: m.status_working, dot: 'bg-status-working' },
  waitingForInput: {
    label: m.status_waiting_for_input,
    dot: 'bg-status-waiting'
  },
  error: { label: m.status_error, dot: 'bg-status-error' }
}

type ThreadCardProps = {
  thread: Thread
  selected: boolean
  onSelect: (threadId: string) => void
}

export const ThreadCard = memo(function ThreadCard({
  thread,
  selected,
  onSelect
}: ThreadCardProps) {
  const status = thread.status === 'idle' ? null : STATUS[thread.status]

  return (
    <button
      type='button'
      aria-current={selected || undefined}
      onClick={() => onSelect(thread.id)}
      className={cn(
        'border-sidebar-border hover:bg-sidebar-accent/60 focus-visible:ring-sidebar-ring flex w-full flex-col gap-1 rounded-lg border px-2.5 py-2 text-left outline-none focus-visible:ring-2',
        selected && 'border-sidebar-foreground/40 bg-sidebar-accent'
      )}
    >
      <span className='truncate text-sm font-medium'>
        {thread.title || m.untitled_thread()}
      </span>
      <span className='text-muted-foreground flex min-w-0 items-center gap-2 text-xs'>
        {status ? (
          <span className='text-sidebar-foreground flex shrink-0 items-center gap-1.5'>
            <span className={cn('size-1.5 rounded-full', status.dot)} />
            {status.label()}
          </span>
        ) : (
          <span className='shrink-0 tabular-nums'>
            {formatRelativeTime(thread.updatedAt, new Date(), getLocale())}
          </span>
        )}
        {thread.branch ? (
          <span className='flex min-w-0 items-center gap-1'>
            <HugeiconsIcon
              icon={GitBranchIcon}
              strokeWidth={2}
              className='size-3 shrink-0'
            />
            <span className='truncate'>{thread.branch}</span>
          </span>
        ) : null}
      </span>
    </button>
  )
})
