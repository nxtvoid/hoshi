import { useState } from 'react'
import { useNow } from './use-now'
import { m } from '@/paraglide/messages'
import { cn, formatElapsed } from '@hoshi/utils'
import { SubagentRow } from './SubagentRow'
import { DelegationRow } from './DelegationRows'
import { useAppStore } from '@/state/app-store'
import { selectQuietThreads } from '@/state/quiet-threads'
import { HugeiconsIcon } from '@hugeicons/react'
import { AgentAvatar, isActive } from './AgentLine'
import { useBusyThreads } from './use-busy-threads'
import { useRequestStore } from '@/state/request-store'
import { ArrowDown01Icon } from '@hugeicons/core-free-icons'
import { type AgentState, subagentState, taskState } from '@/lib/agents'
import {
  delegationOf,
  isDelegation,
  type ToolCallItem,
  type TranscriptItem
} from '@/lib/tool-calls'

const STATES: AgentState[] = ['running', 'waiting', 'done', 'stopped', 'failed']

const COUNTS: Record<AgentState, (count: number) => string> = {
  running: (count) => m.agents_running({ count }),
  waiting: (count) => m.agents_waiting({ count }),
  done: (count) => m.agents_done({ count }),
  stopped: (count) => m.agents_stopped({ count }),
  failed: (count) => m.agents_failed({ count })
}

/** How many agents the card's header shows one by one; the rest are counted. */
const MAX_AVATARS = 5

function isTask(item: TranscriptItem): boolean {
  return isDelegation(item)
}

/**
 * The agents a thread started together, subagents and delegated tasks alike, as one card
 * that says how they stand and opens on each of them.
 */
export function AgentGroup({ items }: { items: ToolCallItem[] }) {
  const threads = useAppStore(selectQuietThreads)
  const pendingTasks = useRequestStore((state) => state.pendingTasks)
  const busy = useBusyThreads()
  const [open, setOpen] = useState(false)
  const agents = items.map((item) => {
    const { id } = item
    if (!isTask(item)) {
      const state: AgentState | null = subagentState(item.content)
      return { id, state, startedAt: item.createdAt }
    }
    const { taskId, error } = delegationOf(item.content)
    const child = taskId
      ? threads.find((thread) => thread.id === taskId)
      : undefined
    const state: AgentState | null = child
      ? taskState(child, pendingTasks, busy)
      : error
        ? 'failed'
        : null
    return { id, state, startedAt: child?.createdAt ?? item.createdAt }
  })
  const active = agents.filter(({ state }) => isActive(state))
  const now = useNow(active.length > 0)
  const counts = STATES.flatMap((state) => {
    const count = agents.filter((agent) => agent.state === state).length
    return count > 0 ? [COUNTS[state](count)] : []
  })
  const started = Math.min(
    ...active.map(({ startedAt }) => Date.parse(startedAt))
  )
  return (
    <div className='border-border/70 rounded-xl border'>
      <button
        type='button'
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className={cn(
          'hover:bg-foreground/5 focus-visible:ring-ring flex w-full min-w-0 cursor-pointer items-center gap-2.5 rounded-xl px-3 py-2 text-left text-sm transition-colors outline-none focus-visible:ring-2',
          open && 'rounded-b-none'
        )}
      >
        <span className='flex shrink-0 items-center gap-1.5'>
          <span className='flex flex-row-reverse -space-x-1 space-x-reverse'>
            {agents
              .slice(0, MAX_AVATARS)
              .reverse()
              .map((agent) => (
                <AgentAvatar
                  key={agent.id}
                  state={agent.state}
                  className='ring-background ring-2'
                />
              ))}
          </span>
          {agents.length > MAX_AVATARS ? (
            <span className='text-muted-foreground text-xs tabular-nums'>
              {m.agents_more({ count: agents.length - MAX_AVATARS })}
            </span>
          ) : null}
        </span>
        <span className='flex min-w-0 flex-1 flex-col'>
          <span className='text-foreground truncate font-medium'>
            {m.agents_count({ count: items.length })}
          </span>
          <span
            className={cn(
              'truncate text-xs',
              active.length > 0
                ? 'text-status-working'
                : 'text-muted-foreground'
            )}
          >
            {counts.join(' · ')}
          </span>
        </span>
        {Number.isFinite(started) ? (
          <span className='text-muted-foreground shrink-0 text-xs tabular-nums'>
            {formatElapsed(Math.max(0, now - started))}
          </span>
        ) : null}
        <HugeiconsIcon
          icon={ArrowDown01Icon}
          strokeWidth={2}
          className={cn(
            'text-muted-foreground size-3.5 shrink-0 transition-transform',
            open && 'rotate-180'
          )}
        />
      </button>
      {open ? (
        <ol className='border-border/70 flex flex-col border-t p-1'>
          {items.map((item) => (
            <li key={item.id} data-item-id={item.id}>
              {isTask(item) ? (
                <DelegationRow item={item} grouped />
              ) : (
                <SubagentRow item={item} grouped />
              )}
            </li>
          ))}
        </ol>
      ) : null}
    </div>
  )
}
