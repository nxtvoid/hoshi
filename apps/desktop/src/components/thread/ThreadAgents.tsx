import { useMemo, useState } from 'react'
import { cn, formatRelativeTime } from '@hoshi/utils'
import { m } from '@/paraglide/messages'
import { getLocale } from '@/paraglide/runtime'
import { HugeiconsIcon } from '@hugeicons/react'
import { useAppStore } from '@/state/app-store'
import { useRequestStore } from '@/state/request-store'
import { useTranscriptStore } from '@/state/transcript-store'
import type { ThreadItem } from '@/bindings/ThreadItem'
import { type AgentState, type ThreadAgent, threadAgents } from '@/lib/agents'
import {
  ArrowRight01Icon,
  BotIcon,
  GitForkIcon
} from '@hugeicons/core-free-icons'

const ROW =
  'flex h-8 w-full min-w-0 cursor-pointer items-center gap-2.5 rounded-lg px-2.5 text-left text-[13px] font-medium text-foreground/80 outline-none transition-colors hover:bg-foreground/5 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset'
const NO_ITEMS: ThreadItem[] = []

/** The color of the dot that tells how an agent stands. */
export const AGENT_DOT: Record<AgentState, string> = {
  running: 'bg-status-working',
  waiting: 'bg-status-waiting',
  done: 'bg-status-done',
  stopped: 'bg-muted-foreground/50',
  failed: 'bg-status-error'
}

function isActive(state: AgentState) {
  return state === 'running' || state === 'waiting'
}

/** The agents that worked for a thread: what it delegated, and its subagents. */
export function ThreadAgents({
  threadId,
  onNavigate
}: {
  threadId: string
  onNavigate?: () => void
}) {
  const threads = useAppStore((state) => state.threads)
  const selectThread = useAppStore((state) => state.selectThread)
  const items = useTranscriptStore((state) =>
    state.threadId === threadId ? state.items : NO_ITEMS
  )
  const [previousOpen, setPreviousOpen] = useState(false)
  const pendingTasks = useRequestStore((state) => state.pendingTasks)
  const agents = useMemo(
    () => threadAgents(threadId, threads, items, pendingTasks),
    [threadId, threads, items, pendingTasks]
  )
  const parent = useMemo(() => {
    const own = threads.find((thread) => thread.id === threadId)
    return threads.find((thread) => thread.id === own?.parentThreadId)
  }, [threadId, threads])
  if (agents.length === 0 && !parent) return null

  const open = (agent: ThreadAgent) => {
    if (agent.kind === 'task') {
      selectThread(agent.id)
      onNavigate?.()
      return
    }
    document
      .querySelector(`[data-item-id="${CSS.escape(agent.id)}"]`)
      ?.scrollIntoView({ block: 'center', behavior: 'smooth' })
  }
  const active = agents.filter((agent) => isActive(agent.state))
  const previous = agents.filter((agent) => !isActive(agent.state))
  const row = (agent: ThreadAgent) => (
    <button
      key={agent.id}
      type='button'
      className={ROW}
      onClick={() => open(agent)}
    >
      <span className='relative flex shrink-0'>
        <HugeiconsIcon
          icon={agent.kind === 'task' ? GitForkIcon : BotIcon}
          strokeWidth={2}
          className='text-muted-foreground size-4'
        />
        <span
          className={cn(
            'ring-popover absolute -right-0.5 -bottom-0.5 size-1.5 rounded-full ring-2',
            AGENT_DOT[agent.state],
            agent.state === 'running' && 'animate-pulse'
          )}
        />
      </span>
      <span className='min-w-0 flex-1 truncate'>{agent.title}</span>
      <span className='text-muted-foreground shrink-0 text-xs font-normal tabular-nums'>
        {formatRelativeTime(agent.at, new Date(), getLocale())}
      </span>
    </button>
  )

  return (
    <>
      <div className='bg-border/70 mx-2.5 my-1 h-px' />
      <p className='text-muted-foreground px-2.5 pt-1 pb-0.5 text-xs'>
        {m.agents()}
      </p>
      {parent ? (
        <button
          type='button'
          className={ROW}
          onClick={() => {
            selectThread(parent.id)
            onNavigate?.()
          }}
        >
          <HugeiconsIcon
            icon={GitForkIcon}
            strokeWidth={2}
            className='text-muted-foreground size-4 shrink-0'
          />
          <span className='min-w-0 flex-1 truncate'>
            {m.delegated_by({ thread: parent.title })}
          </span>
        </button>
      ) : null}
      {active.length > 0 ? (
        <div className='max-h-60 overflow-y-auto'>{active.map(row)}</div>
      ) : null}
      {previous.length > 0 ? (
        <>
          <button
            type='button'
            aria-expanded={previousOpen}
            className={cn(ROW, 'text-muted-foreground font-normal')}
            onClick={() => setPreviousOpen((value) => !value)}
          >
            <span className='min-w-0 flex-1 truncate'>
              {m.agents_previous()}
            </span>
            <span className='text-xs tabular-nums'>{previous.length}</span>
            <HugeiconsIcon
              icon={ArrowRight01Icon}
              strokeWidth={2}
              className={cn(
                'size-3.5 shrink-0 transition-transform',
                previousOpen && 'rotate-90'
              )}
            />
          </button>
          {previousOpen ? (
            <div className='max-h-48 overflow-y-auto'>{previous.map(row)}</div>
          ) : null}
        </>
      ) : null}
    </>
  )
}
