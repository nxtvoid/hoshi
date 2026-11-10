import { useNow } from './use-now'
import { m } from '@/paraglide/messages'
import { AGENT_DOT } from './ThreadAgents'
import { modelRun } from '@/lib/model-runs'
import { WorkLine } from './BackgroundWork'
import { confirm } from '@/state/dialog-store'
import { getLocale } from '@/paraglide/runtime'
import { ModelRunLabel } from './ModelRunLabel'
import { useAppStore } from '@/state/app-store'
import type { Thread } from '@/bindings/Thread'
import { HugeiconsIcon } from '@hugeicons/react'
import { useEffect, useRef, useState } from 'react'
import { Button } from '@hoshi/ui/components/button'
import { useModelsStore } from '@/state/models-store'
import { useRequestStore } from '@/state/request-store'
import { useWholeToolCall } from './use-whole-tool-call'
import { Markdown } from '@/components/markdown/Markdown'
import { ClaudeIcon } from '@/components/brand/ProviderIcon'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { cn, formatClockTime, formatElapsed } from '@hoshi/utils'
import {
  delegationOf,
  reasoningPreview,
  type ToolCallItem
} from '@/lib/tool-calls'
import {
  type AgentState,
  isPlanningWithUser,
  plainPreview,
  runningTasks,
  taskState,
  taskTime
} from '@/lib/agents'
import {
  Alert02Icon,
  ArrowRight01Icon,
  CancelCircleIcon,
  CheckmarkCircle02Icon,
  GitForkIcon,
  Loading03Icon,
  PencilEdit01Icon,
  SentIcon,
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

/** The agent a task runs on, with a dot for how it stands. */
function TaskAvatar({ state }: { state: AgentState | null }) {
  return (
    <span
      aria-hidden
      className='border-border/70 bg-muted relative inline-flex size-6 shrink-0 items-center justify-center rounded-full border'
    >
      <ClaudeIcon className='text-provider-claude size-3.5' />
      {state ? (
        <span
          className={cn(
            'ring-background absolute -right-px -bottom-px size-2 rounded-full ring-2',
            AGENT_DOT[state],
            state === 'running' && 'animate-pulse'
          )}
        />
      ) : null}
    </span>
  )
}

const STATUS_ICONS = {
  running: { icon: Loading03Icon, tone: 'animate-spin' },
  waiting: { icon: UserQuestion01Icon, tone: 'text-status-waiting' },
  done: { icon: CheckmarkCircle02Icon, tone: '' },
  stopped: { icon: CancelCircleIcon, tone: '' },
  failed: { icon: Alert02Icon, tone: 'text-destructive' }
} as const

function statusLabel(
  state: AgentState,
  child: Thread | undefined,
  now: number
) {
  if (!child) return m.tool_failed()
  const time = taskTime(child, state, now)
  if (time.kind === 'elapsed') {
    const elapsed = formatElapsed(time.milliseconds)
    return state === 'waiting'
      ? m.delegation_waiting_for({ elapsed })
      : m.delegation_running_for({ elapsed })
  }
  const clock = formatClockTime(time.at, getLocale())
  return state === 'stopped'
    ? m.delegation_stopped_at({ time: clock })
    : state === 'failed'
      ? m.delegation_failed_at({ time: clock })
      : m.delegation_finished_at({ time: clock })
}

/** How the task's thread is doing: how long it has run, or when it ended. */
function TaskStatus({
  state,
  child
}: {
  state: AgentState
  child: Thread | undefined
}) {
  const now = useNow(state === 'running' || state === 'waiting')
  const { icon, tone } = STATUS_ICONS[state]
  return (
    <span className='text-muted-foreground flex shrink-0 items-center gap-1 text-xs tabular-nums'>
      <HugeiconsIcon
        icon={icon}
        strokeWidth={2}
        className={cn('size-3.5', tone)}
      />
      {statusLabel(state, child, now)}
    </span>
  )
}

const CARD =
  'border-border/70 flex w-full items-center gap-2.5 rounded-xl border px-3 py-2 text-left text-sm transition-colors'

/**
 * A task the agent delegated to a child thread, with how the child is doing and what it
 * said last. It opens the child.
 */
export function DelegationRow({ item }: { item: ToolCallItem }) {
  const whole = useWholeToolCall(item, true)
  const { title, taskId, error } = delegationOf(whole)
  const child = useThread(taskId)
  const pendingTasks = useRequestStore((state) => state.pendingTasks)
  const preview = useRequestStore((state) =>
    taskId ? state.taskPreviews[taskId] : undefined
  )
  const selectThread = useAppStore((state) => state.selectThread)
  const planning = child ? isPlanningWithUser(child, pendingTasks) : false
  const catalog = useModelsStore((state) => state.catalog)
  const run = child ? modelRun(catalog, child.model, child.effort) : null
  const state: AgentState | null = child
    ? taskState(child, pendingTasks)
    : error
      ? 'failed'
      : null
  const name = child?.title ?? title
  const detail = error ?? (preview ? plainPreview(preview) : '')
  const body = (
    <>
      <TaskAvatar state={state} />
      <span className='flex min-w-0 flex-1 flex-col'>
        <span className='text-foreground truncate font-medium'>{name}</span>
        <span
          className={cn(
            'truncate text-xs',
            error ? 'text-destructive' : 'text-muted-foreground'
          )}
        >
          {detail || m.delegation_task()}
        </span>
      </span>
      <ModelRunLabel run={run} />
      {planning || !state ? null : <TaskStatus state={state} child={child} />}
      {child ? (
        <HugeiconsIcon
          icon={ArrowRight01Icon}
          strokeWidth={2}
          className='text-muted-foreground/60 group-hover/task:text-foreground size-3.5 shrink-0 transition-colors'
        />
      ) : null}
    </>
  )
  const card = child ? (
    <button
      type='button'
      aria-label={m.delegation_open_task({ task: name })}
      onClick={() => selectThread(child.id)}
      className={cn(
        CARD,
        'group/task hover:bg-foreground/5 focus-visible:ring-ring cursor-pointer outline-none focus-visible:ring-2',
        planning && 'bg-background relative'
      )}
    >
      {body}
    </button>
  ) : (
    <div className={CARD}>{body}</div>
  )
  if (!child || !planning) return card
  return (
    <div>
      {card}
      <PlanningTray threadId={child.id} />
    </div>
  )
}

/**
 * At the end of the transcript, while its agent rests: the tasks it delegated that still
 * run, whose answers it waits for.
 */
export function TaskWaits({ threadId }: { threadId: string }) {
  const pendingTasks = useRequestStore((state) => state.pendingTasks)
  const ids = useAppStore((state) =>
    runningTasks(threadId, state.threads, pendingTasks)
      .map((thread) => thread.id)
      .join(' ')
  )
  if (!ids) return null
  return (
    <li className='flex flex-col gap-2'>
      {ids.split(' ').map((id) => (
        <TaskWaitLine key={id} threadId={id} />
      ))}
    </li>
  )
}

function TaskWaitLine({ threadId }: { threadId: string }) {
  const child = useThread(threadId)
  const selectThread = useAppStore((state) => state.selectThread)
  const cancelTask = useRequestStore((state) => state.cancelTask)
  const [stopping, setStopping] = useState(false)
  const now = useNow()
  if (!child) return null
  const time = taskTime(child, 'running', now)
  return (
    <WorkLine
      label={m.background_waiting()}
      title={child.title || m.delegation_task()}
      elapsed={time.kind === 'elapsed' ? formatElapsed(time.milliseconds) : ''}
    >
      <Button
        size='xs'
        variant='ghost'
        onClick={() => selectThread(threadId)}
        className='text-muted-foreground hover:text-foreground -ml-1 shrink-0'
      >
        {m.delegation_open()}
      </Button>
      <Button
        size='xs'
        variant='ghost'
        disabled={stopping}
        onClick={() => {
          setStopping(true)
          void cancelTask(threadId)
        }}
        className='text-muted-foreground hover:text-foreground -ml-2 shrink-0'
      >
        {stopping ? m.background_task_stopping() : m.background_task_stop()}
      </Button>
    </WorkLine>
  )
}

/**
 * Under a task's card, while the task waits for the user's changes to its plan: what it
 * waits for, and the ways to answer. Says when it is on screen, so the composer needn't.
 */
function PlanningTray({ threadId }: { threadId: string }) {
  const selectThread = useAppStore((state) => state.selectThread)
  const sendTaskAnswer = useRequestStore((state) => state.sendTaskAnswer)
  const cancelTask = useRequestStore((state) => state.cancelTask)
  const setOnScreen = useRequestStore((state) => state.setTaskOnScreen)
  const ref = useRef<HTMLDivElement>(null)
  const title = useThread(threadId)?.title ?? ''
  const cancel = async () => {
    const confirmed = await confirm({
      title: m.delegation_cancel_title({ task: title }),
      description: m.delegation_cancel_description(),
      confirmLabel: m.delegation_cancel(),
      cancelLabel: m.delegation_cancel_keep(),
      tone: 'destructive',
      icon: CancelCircleIcon
    })
    if (confirmed) await cancelTask(threadId)
  }

  useEffect(() => {
    const element = ref.current
    if (!element) return
    const observer = new IntersectionObserver(([entry]) =>
      setOnScreen(threadId, entry?.isIntersecting ?? false)
    )
    observer.observe(element)
    return () => {
      observer.disconnect()
      setOnScreen(threadId, false)
    }
  }, [threadId, setOnScreen])

  return (
    <div
      ref={ref}
      className='border-status-waiting/20 bg-status-waiting/5 mx-3.5 -mt-2 flex items-center gap-1 rounded-b-lg border border-t-0 pt-3 pr-1.5 pb-1 pl-3 text-xs'
    >
      <HugeiconsIcon
        icon={UserQuestion01Icon}
        strokeWidth={2}
        className='text-status-waiting size-3.5 shrink-0'
      />
      <span className='text-status-waiting/90 ms-1 min-w-0 flex-1 truncate'>
        {m.delegation_waiting_changes()}
      </span>
      <IconButton
        label={m.delegation_cancel()}
        side='top'
        className='bg-destructive/10 text-destructive hover:bg-destructive/15 hover:text-destructive'
        onClick={() => void cancel()}
      >
        <HugeiconsIcon icon={CancelCircleIcon} strokeWidth={2} />
      </IconButton>
      <IconButton
        label={m.delegation_send_plan()}
        side='top'
        className='bg-send/10 text-send hover:bg-send/15 hover:text-send'
        onClick={() => void sendTaskAnswer(threadId)}
      >
        <HugeiconsIcon icon={SentIcon} strokeWidth={2} />
      </IconButton>
      <IconButton
        label={m.delegation_change_plan()}
        side='top'
        className='bg-primary text-primary-foreground hover:bg-primary/90 hover:text-primary-foreground'
        onClick={() => selectThread(threadId)}
      >
        <HugeiconsIcon icon={PencilEdit01Icon} strokeWidth={2} />
      </IconButton>
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
