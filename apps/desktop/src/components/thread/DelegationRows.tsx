import { useNow } from './use-now'
import { m } from '@/paraglide/messages'
import { modelRun } from '@/lib/model-runs'
import { WorkLine } from './BackgroundWork'
import { confirm } from '@/state/dialog-store'
import { AgentLine, AgentStatus } from './AgentLine'
import { useAppStore } from '@/state/app-store'
import { HugeiconsIcon } from '@hugeicons/react'
import { useEffect, useRef, useState } from 'react'
import { Button } from '@hoshi/ui/components/button'
import { useModelsStore } from '@/state/models-store'
import { useRequestStore } from '@/state/request-store'
import { useBusyThreads } from './use-busy-threads'
import { useWholeToolCall } from './use-whole-tool-call'
import { Markdown } from '@/components/markdown/Markdown'
import { IconButton } from '@hoshi/ui/components/icon-button'
import { cn, formatElapsed } from '@hoshi/utils'
import {
  delegationOf,
  type TaskOutcome,
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
  CancelCircleIcon,
  GitForkIcon,
  PencilEdit01Icon,
  SentIcon,
  Tick02Icon,
  UserQuestion01Icon
} from '@hugeicons/core-free-icons'

function useThread(threadId: string | null) {
  return useAppStore((state) =>
    threadId
      ? state.threads.find((thread) => thread.id === threadId)
      : undefined
  )
}

/**
 * A task the agent delegated to a child thread, with how the child is doing and what it
 * said last. It opens the child.
 */
export function DelegationRow({
  item,
  grouped = false
}: {
  item: ToolCallItem
  /** Inside a group's card, which draws the border. */
  grouped?: boolean
}) {
  const whole = useWholeToolCall(item, true)
  const { title, taskId, error } = delegationOf(whole)
  const child = useThread(taskId)
  const pendingTasks = useRequestStore((state) => state.pendingTasks)
  const busy = useBusyThreads()
  const preview = useRequestStore((state) =>
    taskId ? state.taskPreviews[taskId] : undefined
  )
  const selectThread = useAppStore((state) => state.selectThread)
  const cancelTask = useRequestStore((state) => state.cancelTask)
  const [stopping, setStopping] = useState(false)
  const planning = child ? isPlanningWithUser(child, pendingTasks) : false
  const catalog = useModelsStore((state) => state.catalog)
  const run = child ? modelRun(catalog, child.model, child.effort) : null
  const state: AgentState | null = child
    ? taskState(child, pendingTasks, busy)
    : error
      ? 'failed'
      : null
  const name = child?.title ?? title
  const detail = error ?? (preview ? plainPreview(preview) : '')
  const line = (
    <AgentLine
      grouped={grouped}
      state={state}
      title={name}
      detail={detail || m.delegation_task()}
      failed={error !== null}
      tag={m.agent_own_thread()}
      run={run}
      status={
        planning || !state ? null : (
          <AgentStatus
            state={state}
            startedAt={child?.createdAt ?? item.createdAt}
            endedAt={child?.updatedAt ?? null}
          />
        )
      }
      label={m.delegation_open_task({ task: name })}
      onOpen={child ? () => selectThread(child.id) : undefined}
      onStop={
        child && state === 'running'
          ? () => {
              setStopping(true)
              void cancelTask(child.id)
            }
          : undefined
      }
      stopping={stopping}
      className={cn(
        planning && 'bg-background relative',
        planning && grouped && 'rounded-lg'
      )}
    />
  )
  if (!child || !planning) return line
  return (
    <div>
      {line}
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
  const busy = useBusyThreads()
  const ids = useAppStore((state) =>
    runningTasks(threadId, state.threads, pendingTasks, busy)
      .map((thread) => thread.id)
      .join(' ')
  )
  if (!ids) return null
  return (
    <div className='flex flex-col gap-2'>
      {ids.split(' ').map((id) => (
        <TaskWaitLine key={id} threadId={id} />
      ))}
    </div>
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

const NOTE_ICONS: Record<
  TaskOutcome,
  { icon: typeof Tick02Icon; tone: string }
> = {
  completed: { icon: Tick02Icon, tone: 'text-status-done' },
  failed: { icon: Alert02Icon, tone: 'text-destructive' },
  stopped: { icon: CancelCircleIcon, tone: 'text-muted-foreground' }
}

/** The agent's reply to a task's answer, while the work on its tasks goes on. */
export function TaskNote({
  text,
  outcome,
  streaming
}: {
  text: string
  outcome: TaskOutcome
  streaming: boolean
}) {
  const { icon, tone } = NOTE_ICONS[outcome]
  return (
    <div className='flex items-start gap-2'>
      <HugeiconsIcon
        icon={icon}
        strokeWidth={2}
        className={cn('mt-1 size-3.5 shrink-0', tone)}
      />
      <Markdown
        text={text}
        streaming={streaming}
        className='text-muted-foreground min-w-0 flex-1 text-sm'
      />
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
