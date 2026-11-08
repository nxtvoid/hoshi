import { useState } from 'react'
import { cn } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { HugeiconsIcon } from '@hugeicons/react'
import { useAppStore } from '@/state/app-store'
import { isPlanningWithUser } from '@/lib/agents'
import { useRequestStore } from '@/state/request-store'
import { Button } from '@hoshi/ui/components/button'
import { useWholeToolCall } from './use-whole-tool-call'
import type { Thread } from '@/bindings/Thread'
import { Markdown } from '@/components/markdown/Markdown'
import {
  delegationOf,
  reasoningPreview,
  type ToolCallItem
} from '@/lib/tool-calls'
import {
  Alert02Icon,
  ArrowRight01Icon,
  CheckmarkCircle02Icon,
  GitForkIcon,
  Loading03Icon,
  UserQuestion01Icon
} from '@hugeicons/core-free-icons'

function useThread(threadId: string | null) {
  return useAppStore((state) =>
    threadId
      ? state.threads.find((thread) => thread.id === threadId)
      : undefined
  )
}

function OpenButton({ threadId }: { threadId: string }) {
  const selectThread = useAppStore((state) => state.selectThread)
  return (
    <Button
      size='xs'
      variant='ghost'
      className='shrink-0'
      onClick={() => selectThread(threadId)}
    >
      {m.delegation_open()}
      <HugeiconsIcon icon={ArrowRight01Icon} strokeWidth={2} />
    </Button>
  )
}

function TaskStatus({
  child,
  planning
}: {
  child: Thread | undefined
  planning: boolean
}) {
  if (!child) return null
  const { icon, label, tone } = planning
    ? {
        icon: UserQuestion01Icon,
        label: m.delegation_waiting_changes(),
        tone: 'text-status-waiting'
      }
    : child.status === 'working'
      ? { icon: Loading03Icon, label: m.tool_running(), tone: 'animate-spin' }
      : child.status === 'waitingForInput'
        ? {
            icon: UserQuestion01Icon,
            label: m.status_waiting_for_input(),
            tone: 'text-status-waiting'
          }
        : child.status === 'error'
          ? {
              icon: Alert02Icon,
              label: m.tool_failed(),
              tone: 'text-destructive'
            }
          : {
              icon: CheckmarkCircle02Icon,
              label: m.delegation_done(),
              tone: ''
            }
  return (
    <span className='text-muted-foreground flex shrink-0 items-center gap-1 text-xs'>
      <HugeiconsIcon
        icon={icon}
        strokeWidth={2}
        className={cn('size-3.5', tone)}
      />
      {label}
    </span>
  )
}

/** A task the agent delegated to a child thread, with how the child is doing. */
export function DelegationRow({ item }: { item: ToolCallItem }) {
  const whole = useWholeToolCall(item, true)
  const { title, taskId, error } = delegationOf(whole)
  const child = useThread(taskId)
  const pendingTasks = useRequestStore((state) => state.pendingTasks)
  const sendTaskAnswer = useRequestStore((state) => state.sendTaskAnswer)
  const planning = child ? isPlanningWithUser(child, pendingTasks) : false
  return (
    <div className='border-border/70 flex items-center gap-2.5 rounded-xl border px-3 py-2 text-sm'>
      <HugeiconsIcon
        icon={GitForkIcon}
        strokeWidth={2}
        className='text-muted-foreground size-4 shrink-0'
      />
      <span className='flex min-w-0 flex-1 flex-col'>
        <span className='text-foreground truncate font-medium'>
          {child?.title ?? title}
        </span>
        <span className='text-muted-foreground truncate text-xs'>
          {error ?? m.delegation_task()}
        </span>
      </span>
      <TaskStatus child={child} planning={planning} />
      {child && planning ? (
        <Button
          size='xs'
          variant='ghost'
          className='text-muted-foreground shrink-0'
          onClick={() => void sendTaskAnswer(child.id)}
        >
          {m.delegation_send_plan()}
        </Button>
      ) : null}
      {child ? <OpenButton threadId={child.id} /> : null}
    </div>
  )
}

/**
 * A delegated task's answer, which hoshi handed back to the thread. It starts folded: the
 * agent's reply that follows usually tells the same.
 */
export function TaskResultRow({
  text,
  childThreadId
}: {
  text: string
  childThreadId: string
}) {
  const child = useThread(childThreadId)
  const [open, setOpen] = useState(false)
  const answer = text.slice(text.indexOf('\n') + 1).trim()
  return (
    <div className='border-border/70 flex flex-col gap-2 rounded-xl border px-3 py-2'>
      <div className='flex items-center gap-2.5 text-sm'>
        <button
          type='button'
          aria-expanded={open}
          disabled={!answer}
          onClick={() => setOpen((value) => !value)}
          className='focus-visible:ring-ring flex min-w-0 flex-1 cursor-pointer items-center gap-2.5 rounded-md text-left outline-none focus-visible:ring-2'
        >
          <HugeiconsIcon
            icon={GitForkIcon}
            strokeWidth={2}
            className='text-muted-foreground size-4 shrink-0'
          />
          <span className='text-foreground shrink-0 font-medium'>
            {m.task_result_from({ task: child?.title ?? '' })}
          </span>
          {open ? null : (
            <span className='text-muted-foreground min-w-0 flex-1 truncate'>
              {reasoningPreview(answer, false)}
            </span>
          )}
          {answer ? (
            <HugeiconsIcon
              icon={ArrowRight01Icon}
              strokeWidth={2}
              className={cn(
                'text-muted-foreground ms-auto size-3.5 shrink-0 transition-transform',
                open && 'rotate-90'
              )}
            />
          ) : null}
        </button>
        {child ? <OpenButton threadId={child.id} /> : null}
      </div>
      {open && answer ? (
        <div className='bg-muted/40 max-h-60 overflow-auto rounded-md px-3 py-2'>
          <Markdown text={answer} className='text-sm' />
        </div>
      ) : null}
    </div>
  )
}

/** In a delegated task's thread: which thread delegated it, to go back. */
export function DelegatedBanner({
  parentThreadId
}: {
  parentThreadId: string
}) {
  const parent = useThread(parentThreadId)
  const selectThread = useAppStore((state) => state.selectThread)
  if (!parent) return null
  return (
    <div className='text-muted-foreground flex items-center gap-2 text-xs'>
      <HugeiconsIcon icon={GitForkIcon} strokeWidth={2} className='size-3.5' />
      <span className='min-w-0 truncate'>
        {m.delegated_by({ thread: parent.title })}
      </span>
      <Button size='xs' variant='ghost' onClick={() => selectThread(parent.id)}>
        {m.delegation_back()}
      </Button>
    </div>
  )
}
