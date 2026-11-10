import { useNow } from './use-now'
import { m } from '@/paraglide/messages'
import { AGENT_DOT } from './ThreadAgents'
import { getLocale } from '@/paraglide/runtime'
import { ModelRunLabel } from './ModelRunLabel'
import type { AgentState } from '@/lib/agents'
import { HugeiconsIcon } from '@hugeicons/react'
import type { ModelRun } from '@/lib/model-runs'
import { Button } from '@hoshi/ui/components/button'
import { ClaudeIcon } from '@/components/brand/ProviderIcon'
import { cn, formatClockTime, formatElapsed } from '@hoshi/utils'
import {
  Alert02Icon,
  ArrowUpRight01Icon,
  CancelCircleIcon,
  CheckmarkCircle02Icon,
  Loading03Icon,
  UserQuestion01Icon
} from '@hugeicons/core-free-icons'

/** The agent something runs on, with a dot for how it stands. */
export function AgentAvatar({
  state,
  className
}: {
  state: AgentState | null
  className?: string
}) {
  return (
    <span
      aria-hidden
      className={cn(
        'border-border/70 bg-muted relative inline-flex size-6 shrink-0 items-center justify-center rounded-full border',
        className
      )}
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

export function isActive(state: AgentState | null) {
  return state === 'running' || state === 'waiting'
}

function statusLabel(
  state: AgentState,
  startedAt: string,
  endedAt: string | null,
  now: number
) {
  if (isActive(state)) {
    const started = Date.parse(startedAt)
    const elapsed = formatElapsed(
      Number.isFinite(started) ? Math.max(0, now - started) : 0
    )
    return state === 'waiting'
      ? m.delegation_waiting_for({ elapsed })
      : m.delegation_running_for({ elapsed })
  }
  const time = endedAt ? formatClockTime(endedAt, getLocale()) : null
  switch (state) {
    case 'stopped':
      return time ? m.delegation_stopped_at({ time }) : m.agent_stopped()
    case 'failed':
      return time ? m.delegation_failed_at({ time }) : m.agent_failed()
    default:
      return time ? m.delegation_finished_at({ time }) : m.agent_finished()
  }
}

/** How an agent is doing: how long it has run, or how it ended, with when if known. */
export function AgentStatus({
  state,
  startedAt,
  endedAt
}: {
  state: AgentState
  startedAt: string
  endedAt: string | null
}) {
  const now = useNow(isActive(state))
  const { icon, tone } = STATUS_ICONS[state]
  return (
    <span className='text-muted-foreground flex shrink-0 items-center gap-1 text-xs tabular-nums'>
      <HugeiconsIcon
        icon={icon}
        strokeWidth={2}
        className={cn('size-3.5', tone)}
      />
      {statusLabel(state, startedAt, endedAt, now)}
    </span>
  )
}

/**
 * One agent working for the thread, a subagent or a delegated task alike: who it is, what
 * it does or said last, what it runs on and how it stands. It opens the agent's own view.
 * Alone it is a card; `grouped`, a line of its group's card.
 */
export function AgentLine({
  grouped,
  state,
  title,
  kind,
  detail,
  failed = false,
  tag,
  run,
  status,
  label,
  onOpen,
  onStop,
  stopping = false,
  className
}: {
  grouped: boolean
  state: AgentState | null
  title: string
  /** What kind of subagent it is, beside its title. */
  kind?: string
  detail?: string
  failed?: boolean
  tag?: string
  run: ModelRun | null
  status?: React.ReactNode
  label: string
  onOpen?: () => void
  onStop?: () => void
  stopping?: boolean
  className?: string
}) {
  return (
    <div
      className={cn(
        '@container/agent relative flex w-full items-center gap-2.5 text-sm',
        grouped
          ? 'rounded-lg px-2 py-1.5'
          : 'border-border/70 rounded-xl border px-3 py-2',
        onOpen && 'hover:bg-foreground/5 transition-colors',
        className
      )}
    >
      {onOpen ? (
        <button
          type='button'
          aria-label={label}
          onClick={onOpen}
          className='focus-visible:ring-ring absolute inset-0 cursor-pointer rounded-[inherit] outline-none focus-visible:ring-2'
        />
      ) : null}
      <AgentAvatar state={state} />
      <span className='flex min-w-0 flex-1 flex-col'>
        <span className='flex min-w-0 items-center gap-2'>
          <span className='text-foreground truncate font-medium'>{title}</span>
          {kind ? (
            <span className='text-muted-foreground hidden shrink-0 text-xs @lg/agent:inline'>
              {kind}
            </span>
          ) : null}
        </span>
        {detail ? (
          <span
            className={cn(
              'truncate text-xs',
              failed ? 'text-destructive' : 'text-muted-foreground'
            )}
          >
            {detail}
          </span>
        ) : null}
      </span>
      {tag ? (
        <span className='border-border/70 text-muted-foreground hidden shrink-0 rounded-full border px-1.5 text-xs @lg/agent:inline'>
          {tag}
        </span>
      ) : null}
      <span className='hidden @2xl/agent:contents'>
        <ModelRunLabel run={run} />
      </span>
      {status}
      {onStop ? (
        <Button
          size='xs'
          variant='ghost'
          disabled={stopping}
          onClick={onStop}
          className='text-muted-foreground hover:text-foreground relative -mx-1 shrink-0'
        >
          {stopping ? m.background_task_stopping() : m.background_task_stop()}
        </Button>
      ) : null}
      {onOpen ? (
        <HugeiconsIcon
          icon={ArrowUpRight01Icon}
          strokeWidth={2}
          className='text-muted-foreground/70 size-3.5 shrink-0'
        />
      ) : null}
    </div>
  )
}
